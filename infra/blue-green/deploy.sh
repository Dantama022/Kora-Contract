#!/usr/bin/env bash
# =============================================================================
# Kora Protocol — Blue-Green Deployment
#
# Deploys the API gateway and indexer services with zero (or near-zero) downtime.
#
# Strategy:
#   1. Stand up the NEW version alongside the OLD (blue/green swap)
#   2. Health-check the new version until it passes
#   3. Shift traffic: update the load balancer / nginx upstream to point at green
#   4. Drain in-flight requests from blue (grace period)
#   5. If health checks fail post-cutover → automated rollback to blue
#   6. Tear down old version after successful stabilisation
#
# Handles:
#   - In-flight GraphQL subscriptions: nginx proxy_read_timeout + reconnect hint
#   - Indexer double-processing prevention: new indexer starts from the
#     cursor/sequence watermark recorded by the outgoing instance, not from 0
#   - Automatic rollback on health check failure
#
# Usage:
#   ./infra/blue-green/deploy.sh [api|indexer|both] [--rollback]
#
# Environment:
#   DOCKER_IMAGE_API      Full docker image reference for api service
#   DOCKER_IMAGE_INDEXER  Full docker image reference for indexer service
#   COMPOSE_FILE          Path to docker-compose.yml (default: infra/blue-green/docker-compose.yml)
#   HEALTH_CHECK_RETRIES  Number of health check attempts before rollback (default: 12)
#   HEALTH_CHECK_INTERVAL Health check interval in seconds (default: 5)
#   DRAIN_WAIT_SECONDS    Seconds to wait for old instance to drain (default: 30)
#
# =============================================================================

set -euo pipefail

SERVICE="${1:-both}"
ROLLBACK_MODE="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${COMPOSE_FILE:-$SCRIPT_DIR/docker-compose.yml}"

HEALTH_CHECK_RETRIES="${HEALTH_CHECK_RETRIES:-12}"
HEALTH_CHECK_INTERVAL="${HEALTH_CHECK_INTERVAL:-5}"
DRAIN_WAIT_SECONDS="${DRAIN_WAIT_SECONDS:-30}"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info()    { echo -e "${YELLOW}[blue-green]${NC} $*"; }
log_success() { echo -e "${GREEN}[blue-green] ✓${NC} $*"; }
log_error()   { echo -e "${RED}[blue-green] ✗${NC} $*" >&2; }

# ── State file: tracks active colour per service ──────────────────────────────

STATE_FILE="$SCRIPT_DIR/.active-colour"
touch "$STATE_FILE"

get_active_colour() {
  local svc="$1"
  grep "^${svc}=" "$STATE_FILE" 2>/dev/null | cut -d= -f2 || echo "blue"
}

set_active_colour() {
  local svc="$1" colour="$2"
  # Update or insert
  if grep -q "^${svc}=" "$STATE_FILE" 2>/dev/null; then
    sed -i "s/^${svc}=.*/${svc}=${colour}/" "$STATE_FILE"
  else
    echo "${svc}=${colour}" >> "$STATE_FILE"
  fi
}

opposite_colour() {
  [ "$1" = "blue" ] && echo "green" || echo "blue"
}

# ── Health check ──────────────────────────────────────────────────────────────

health_check() {
  local name="$1" url="$2"
  log_info "Health-checking $name at $url ..."

  for i in $(seq 1 "$HEALTH_CHECK_RETRIES"); do
    if curl -sf --max-time 5 "$url" > /dev/null 2>&1; then
      log_success "$name is healthy (attempt $i/$HEALTH_CHECK_RETRIES)"
      return 0
    fi
    log_info "  attempt $i/$HEALTH_CHECK_RETRIES failed, waiting ${HEALTH_CHECK_INTERVAL}s ..."
    sleep "$HEALTH_CHECK_INTERVAL"
  done

  log_error "$name failed health check after $HEALTH_CHECK_RETRIES attempts"
  return 1
}

# ── Indexer cursor handoff ─────────────────────────────────────────────────────
#
# Before starting the new indexer, record the outgoing indexer's current
# event cursor/watermark. The new instance will start from this position,
# preventing double-processing of events during the handover window.

record_indexer_cursor() {
  local active_colour="$1"
  local cursor_file="$SCRIPT_DIR/.indexer-cursor"

  log_info "Recording indexer cursor from ${active_colour} instance ..."

  # The indexer exposes its current watermark at /internal/cursor
  local indexer_port
  indexer_port=$([ "$active_colour" = "blue" ] && echo "8081" || echo "8082")

  cursor=$(curl -sf "http://localhost:${indexer_port}/internal/cursor" 2>/dev/null || echo "")
  if [ -n "$cursor" ]; then
    echo "$cursor" > "$cursor_file"
    log_success "Indexer cursor recorded: $cursor"
  else
    log_info "Could not read cursor from ${active_colour} indexer (may not be running yet)"
    rm -f "$cursor_file"
  fi
}

# ── Traffic shift ─────────────────────────────────────────────────────────────
#
# Update nginx to route traffic to the new instance.
# Uses nginx's graceful reload (SIGHUP) — in-flight requests complete before
# the upstream switch takes effect, so existing SSE subscriptions get a
# reconnect hint rather than a hard drop.

shift_traffic() {
  local service="$1" new_colour="$2"
  local new_port

  case "${service}-${new_colour}" in
    api-blue)     new_port=3001 ;;
    api-green)    new_port=3002 ;;
    indexer-blue) new_port=8081 ;;
    indexer-green) new_port=8082 ;;
    health-blue)  new_port=8085 ;;
    health-green) new_port=8086 ;;
    keeper-*|risk-pipeline-*)
      log_info "No reverse-proxy traffic shift needed for worker service: $service"
      return 0
      ;;
    *)
      log_error "Unknown service/colour combo: ${service}/${new_colour}"
      return 1
      ;;
  esac

  log_info "Shifting $service traffic to $new_colour (port $new_port) ..."

  # Write new nginx upstream config
  local nginx_conf="$SCRIPT_DIR/nginx/upstream-${service}.conf"
  cat > "$nginx_conf" <<NGINX
upstream kora_${service} {
    server 127.0.0.1:${new_port};
    keepalive 64;
}
NGINX

  # Graceful nginx reload — does NOT drop existing connections
  if command -v nginx &> /dev/null; then
    nginx -s reload
    log_success "nginx reloaded — traffic shifted to $service/$new_colour"
  else
    log_info "nginx not found locally; assuming managed externally (k8s / docker-compose nginx service)"
    docker compose -f "$COMPOSE_FILE" exec nginx nginx -s reload 2>/dev/null || \
      log_info "nginx reload skipped (not running in compose)"
  fi
}

# ── Deploy a single service ───────────────────────────────────────────────────

deploy_service() {
  local service="$1"
  local active_colour new_colour image_var image_ref

  active_colour=$(get_active_colour "$service")
  new_colour=$(opposite_colour "$active_colour")

  log_info "Deploying $service: $active_colour → $new_colour"

  case "$service" in
    api)           image_var="DOCKER_IMAGE_API" ;;
    indexer)       image_var="DOCKER_IMAGE_INDEXER" ;;
    keeper)        image_var="DOCKER_IMAGE_KEEPER" ;;
    risk-pipeline) image_var="DOCKER_IMAGE_RISK" ;;
    health)        image_var="DOCKER_IMAGE_HEALTH" ;;
    *)
      log_error "Unknown service: $service"
      return 1
      ;;
  esac

  image_ref="${!image_var:-}"
  if [ -z "$image_ref" ]; then
    log_error "$image_var is not set"
    return 1
  fi

  # For indexer: capture cursor before starting new instance
  if [ "$service" = "indexer" ]; then
    record_indexer_cursor "$active_colour"
  fi

  # ── Step 1: Start new instance ────────────────────────────────────────────

  log_info "Starting $service/$new_colour with image $image_ref ..."

  COLOUR="$new_colour" \
  SERVICE_IMAGE="$image_ref" \
  INDEXER_START_CURSOR="$(cat "$SCRIPT_DIR/.indexer-cursor" 2>/dev/null || echo "")" \
  docker compose -f "$COMPOSE_FILE" \
    up -d --no-deps --pull always \
    "${service}-${new_colour}"

  # ── Step 2: Health check new instance ────────────────────────────────────

  local health_url=""
  case "${service}-${new_colour}" in
    api-blue)            health_url="http://localhost:3001/health" ;;
    api-green)           health_url="http://localhost:3002/health" ;;
    indexer-blue)        health_url="http://localhost:8081/health" ;;
    indexer-green)       health_url="http://localhost:8082/health" ;;
    health-blue)         health_url="http://localhost:8085/health" ;;
    health-green)        health_url="http://localhost:8086/health" ;;
    keeper-*)            health_url="${KEEPER_HEALTH_URL:-}" ;;
    risk-pipeline-*)     health_url="${RISK_PIPELINE_HEALTH_URL:-}" ;;
  esac

  if [ -n "$health_url" ]; then
    if ! health_check "${service}/${new_colour}" "$health_url"; then
      log_error "New instance failed health check — rolling back"
      docker compose -f "$COMPOSE_FILE" stop "${service}-${new_colour}"
      docker compose -f "$COMPOSE_FILE" rm -f "${service}-${new_colour}"
      log_info "Rollback complete. $service/$active_colour remains active."
      return 1
    fi
  else
    log_info "ℹ️ ${service}: No HTTP health endpoint configured; skipping probe (not faking success)"
  fi

  # ── Step 3: Shift traffic ─────────────────────────────────────────────────

  shift_traffic "$service" "$new_colour"

  # ── Step 4: Post-cutover health check (automated rollback window) ─────────

  log_info "Post-cutover stabilisation check (${HEALTH_CHECK_RETRIES} × ${HEALTH_CHECK_INTERVAL}s) ..."
  sleep 5 # brief pause for traffic to settle

  local post_health_url=""
  case "$service" in
    api)           post_health_url="http://localhost:80/health" ;; # via nginx
    indexer)       post_health_url="http://localhost:80/indexer/health" ;;
    health)        post_health_url="http://localhost:8085/health" ;;
    keeper|risk-pipeline)
      post_health_url=""
      ;;
  esac

  if [ -n "$post_health_url" ]; then
    if ! health_check "${service} (via nginx)" "$post_health_url"; then
      log_error "Post-cutover health check failed — automated rollback!"
      shift_traffic "$service" "$active_colour"
      sleep 2
      docker compose -f "$COMPOSE_FILE" stop "${service}-${new_colour}"
      docker compose -f "$COMPOSE_FILE" rm -f "${service}-${new_colour}"
      log_info "Automated rollback complete. Traffic restored to $service/$active_colour."
      return 1
    fi
  fi

  # ── Step 5: Record new active colour ─────────────────────────────────────

  set_active_colour "$service" "$new_colour"
  log_success "$service deployed successfully. Active: $new_colour"

  # ── Step 6: Drain and stop old instance ──────────────────────────────────

  log_info "Draining old $service/$active_colour (${DRAIN_WAIT_SECONDS}s grace period) ..."
  sleep "$DRAIN_WAIT_SECONDS"

  docker compose -f "$COMPOSE_FILE" stop "${service}-${active_colour}"
  docker compose -f "$COMPOSE_FILE" rm -f "${service}-${active_colour}"
  log_success "Old $service/$active_colour stopped and removed."
}

# ── Rollback command ──────────────────────────────────────────────────────────

rollback_service() {
  local service="$1"
  local active_colour inactive_colour

  active_colour=$(get_active_colour "$service")
  inactive_colour=$(opposite_colour "$active_colour")

  log_info "Manual rollback: $service $active_colour → $inactive_colour"

  # Start old colour if it's not already running
  docker compose -f "$COMPOSE_FILE" up -d --no-deps "${service}-${inactive_colour}" || true

  if ! health_check "${service}/${inactive_colour}" "http://localhost:$([ "$inactive_colour" = "blue" ] && echo "3001" || echo "3002")/health"; then
    log_error "Rollback target $service/$inactive_colour is not healthy — cannot roll back"
    return 1
  fi

  shift_traffic "$service" "$inactive_colour"
  set_active_colour "$service" "$inactive_colour"

  sleep "$DRAIN_WAIT_SECONDS"
  docker compose -f "$COMPOSE_FILE" stop "${service}-${active_colour}" || true
  docker compose -f "$COMPOSE_FILE" rm -f "${service}-${active_colour}" || true

  log_success "Manual rollback complete. $service is now on $inactive_colour."
}

# ── Main ──────────────────────────────────────────────────────────────────────

echo "=== Kora Protocol — Blue-Green Deployment ==="
echo "Service  : $SERVICE"
echo "Mode     : ${ROLLBACK_MODE:-deploy}"
echo ""

if [ "$ROLLBACK_MODE" = "--rollback" ]; then
  case "$SERVICE" in
    api)           rollback_service api ;;
    indexer)       rollback_service indexer ;;
    keeper)        rollback_service keeper ;;
    risk-pipeline) rollback_service risk-pipeline ;;
    health)        rollback_service health ;;
    both|all)
      rollback_service api
      rollback_service indexer
      rollback_service keeper
      rollback_service risk-pipeline
      rollback_service health
      ;;
  esac
  exit 0
fi

case "$SERVICE" in
  api)           deploy_service api ;;
  indexer)       deploy_service indexer ;;
  keeper)        deploy_service keeper ;;
  risk-pipeline) deploy_service risk-pipeline ;;
  health)        deploy_service health ;;
  both)
    deploy_service api
    deploy_service indexer
    ;;
  all)
    deploy_service api
    deploy_service indexer
    deploy_service keeper
    deploy_service risk-pipeline
    deploy_service health
    ;;
  *)
    log_error "Unknown service: $SERVICE. Use: api | indexer | keeper | risk-pipeline | health | both | all"
    exit 1
    ;;
esac

log_success "Blue-green deployment complete."
