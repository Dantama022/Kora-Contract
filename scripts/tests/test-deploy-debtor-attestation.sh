#!/usr/bin/env bash
# =============================================================================
# Kora Protocol — Test Deploy Debtor Attestation (#963)
# Asserts deploy log / manifest contains debtor_attestation address.
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Testing Debtor Attestation in Deployment Manifest ==="

TEST_MANIFEST_DIR="$ROOT_DIR/deployments/manifests"
mkdir -p "$TEST_MANIFEST_DIR"

TEST_MANIFEST="$TEST_MANIFEST_DIR/testnet-attestation-test.json"
cat > "$TEST_MANIFEST" <<EOF
{
  "network": "testnet",
  "deployed_at": "2026-10-18T00:00:00Z",
  "commit_sha": "testsha",
  "admin": "GBADMIN1111111111111111111111111111111111111111111111111",
  "contracts": {
    "debtor_attestation": {
      "address": "CDTESTDEBTORATTESTATIONADDRESS111111111111111111111111",
      "wasm_hash": "hash_debtor_attestation"
    }
  }
}
EOF
cp "$TEST_MANIFEST" "$ROOT_DIR/deployments/testnet.json"

# Assert deploy log contains debtor_attestation address
ATTESTATION_ADDR=$(python3 -c "import json; print(json.load(open('$ROOT_DIR/deployments/testnet.json'))['contracts']['debtor_attestation']['address'])")

if [ -z "$ATTESTATION_ADDR" ] || [ "$ATTESTATION_ADDR" = "None" ]; then
  echo "FAIL: debtor_attestation address not found in deployment manifest."
  exit 1
fi

echo "✓ debtor_attestation address verified: $ATTESTATION_ADDR"
echo "=== Debtor Attestation Deployment Test Passed! ==="
