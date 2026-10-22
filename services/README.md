# Kora Protocol — Off-Chain Services

This directory contains the off-chain backend services that complement the on-chain Soroban contracts.

| Service | Issue | Type | Description |
|---|---|---|---|
| [`api/gateway/`](api/gateway/) | [#753](https://github.com/OpenLedger-Foundation/Kora-Contract/issues/753) | Node.js | Public API gateway (read routes, authenticated mutations, rate limiting) |
| [`indexer/`](indexer/) | — | Rust | On-chain ledger event ingestion and query indexer |
| [`keeper/`](keeper/) | [#765](https://github.com/Creed1759/Kora-Contract/issues/765) | Node.js | Background job queue for deadline-based contract triggers |
| [`risk-pipeline/`](risk-pipeline/) | [#755](https://github.com/OpenLedger-Foundation/Kora-Contract/issues/755) | Node.js | Off-chain risk scoring data pipeline (suggested scores for verifier review) |
| [`health/`](health/) | [#771](https://github.com/Creed1759/Kora-Contract/issues/771) | Node.js | Unified health aggregation and Prometheus metrics relay |
| [`fx-ingestion/`](fx-ingestion/) | [#766](https://github.com/Creed1759/Kora-Contract/issues/766) | Node.js | FX rate ingestion and oracle relay for African currencies |
| [`audit-log/`](audit-log/) | [#768](https://github.com/Creed1759/Kora-Contract/issues/768) | Node.js | Tamper-evident hash-chained off-chain audit log |
| [`reporting/`](reporting/) | [#770](https://github.com/Creed1759/Kora-Contract/issues/770) | Node.js | Data export service for regulatory/tax reporting |
| [`ipfs-pinning/`](ipfs-pinning/) | [#754](https://github.com/OpenLedger-Foundation/Kora-Contract/issues/754) | Node.js | Redundant IPFS pinning service for invoice metadata |
| [`governance-sync/`](governance-sync/) | — | Node.js | Reads governance-voting proposals and syncs them to a forum |
| [`offchain-voting/`](offchain-voting/) | — | Node.js | Off-chain vote collection using an injected on-chain governance source |
| [`governance-sandbox/`](governance-sandbox/) | — | Node.js | Simulates proposal impact against a supplied protocol snapshot |

---

## Service Commands & Scripts

Before adding deployment or build commands, verify each service's package manager and scripts:

### Node.js Services (`api/gateway`, `keeper`, `risk-pipeline`, `health`)
```bash
# In services/<service-dir>:
npm install
npm test
npm run build
npm start
```

### Rust Services (`indexer`)
```bash
# From workspace root:
cargo test -p kora-indexer
cargo build -p kora-indexer --release
```

---

## Deployment Workflow (Issue #934)

Deployments are automated through [`.github/workflows/deploy-services.yml`](../.github/workflows/deploy-services.yml) using the zero-downtime **Blue-Green Deployment** strategy documented in [`infra/blue-green/README.md`](../infra/blue-green/README.md).

### Workflow Triggers & Safety Rules
1. **Push to `main`**: Automatically triggers pre-flight and pre-deploy test/build steps for paths touching `services/**`.
2. **Manual `workflow_dispatch`**:
   - `service`: `all`, `api`, `indexer`, `keeper`, `risk-pipeline`, or `health`
   - `environment`: `staging` or `production`
   - `image_tag`: Optional commit SHA or Docker tag (defaults to `github.sha`)

### Required Pre-Deploy Checks
Every deployment requires applicable unit tests and compilation builds to pass before any containers are stood up:
- A failed test or build immediately halts the workflow, preventing artifact publication and deployment.

### Required Environment Configuration & Secrets

Production and staging credentials are delivered via GitHub Secrets and workload-specific IAM roles:

| Secret / Config Name | Applicable Services | Purpose |
|---|---|---|
| `DEPLOY_SSH_KEY` | All remote deploys | SSH key for deployment host access |
| `API_DEPLOY_URL` | `api` | Target endpoint for post-deployment health check (`/health`) |
| `INDEXER_DEPLOY_URL` | `indexer` | Target endpoint for indexer health probe (`/health`) |
| `HEALTH_DEPLOY_URL` | `health` | Target endpoint for health aggregator probe (`/health`) |
| `KEEPER_HEALTH_URL` | `keeper` (optional) | HTTP probe for keeper worker (if configured) |
| `RISK_PIPELINE_HEALTH_URL` | `risk-pipeline` (optional) | HTTP probe for risk-pipeline (if configured) |
| `STELLAR_RPC_URL` | `api`, `indexer`, `keeper` | Soroban RPC connection URL |
| `DATABASE_URL` | `api`, `indexer`, `keeper` | Database connection string |
| `JWT_SECRET` | `api` | Secret key for verifying caller JWTs on mutating routes |

### Post-Deployment Health Probes & Unavailable Configurations
- Services with reachable HTTP endpoints (`api`, `indexer`, `health`) are actively probed over multiple retries post-cutover.
- If a service probe fails, the deployment aborts with non-zero exit code to trigger rollback.
- For worker services without an exposed HTTP port (`keeper`, `risk-pipeline`), if no health URL is configured, the workflow explicitly logs:
  `ℹ️ <service>: No HTTP health endpoint configured; skipping probe (not faking success)`
  It never marks an unexecuted health probe as a simulated success.

### Rollback Guidance
- **Automated Rollback**: The deployment script automatically shifts traffic back to the previously active color slot and terminates the failing container if any health probe fails.
- **Manual Rollback**: Run the rollback command:
  ```bash
  ./infra/blue-green/deploy.sh <service> --rollback
  ```
  See detailed rollback procedures in [`infra/blue-green/README.md`](../infra/blue-green/README.md).
