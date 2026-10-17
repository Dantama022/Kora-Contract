# Kora Protocol — Off-Chain Services

This directory contains the off-chain backend services that complement the on-chain Soroban contracts.

| Service | Issue | Description |
|---|---|---|
| [`keeper/`](keeper/) | [#765](https://github.com/Creed1759/Kora-Contract/issues/765) | Background job queue for deadline-based contract triggers |
| [`fx-ingestion/`](fx-ingestion/) | [#766](https://github.com/Creed1759/Kora-Contract/issues/766) | FX rate ingestion and oracle relay for African currencies |
| [`audit-log/`](audit-log/) | [#768](https://github.com/Creed1759/Kora-Contract/issues/768) | Tamper-evident hash-chained off-chain audit log |
| [`reporting/`](reporting/) | [#770](https://github.com/Creed1759/Kora-Contract/issues/770) | Data export service for regulatory/tax reporting |
| [`risk-pipeline/`](risk-pipeline/) | [#755](https://github.com/OpenLedger-Foundation/Kora-Contract/issues/755) | Off-chain risk scoring data pipeline (suggested scores for verifier review) |
| [`ipfs-pinning/`](ipfs-pinning/) | [#754](https://github.com/OpenLedger-Foundation/Kora-Contract/issues/754) | Redundant IPFS pinning service for invoice metadata |
| [`api/gateway/`](api/gateway/) | [#753](https://github.com/OpenLedger-Foundation/Kora-Contract/issues/753) | Public read-only API gateway (invoices, listings, positions, risk scores) |
| [`governance-sync/`](governance-sync/) | — | Reads governance-voting proposals and syncs them to a forum |
| [`offchain-voting/`](offchain-voting/) | — | Off-chain vote collection using an injected on-chain governance source |
| [`governance-sandbox/`](governance-sandbox/) | — | Simulates proposal impact against a supplied protocol snapshot |

These governance services share the deployment manifest written to
`deployments/<network>.json`. The canonical voting contract address is
`contracts.governance_voting.address`. `governance-sync` can load this directly
with `KORA_DEPLOYMENT_MANIFEST`; the injected source used by `offchain-voting`
and snapshot provider used by `governance-sandbox` should use the same field when
wiring their on-chain data adapters.

All services are TypeScript, independently deployable, and share no runtime dependencies on each other. The `audit-log` service is consumed as a library by the others.

Production and testnet credentials are delivered per service and environment
from AWS Secrets Manager using workload-specific read roles. Service code reads
the injected environment variables; it does not load `.env` files. Use only
disposable testnet values for local development. See
[`infra/secrets/`](../infra/secrets/README.md) for isolation, IAM, and rotation
requirements.

## Quick start

```bash
# From any service directory:
npm install
npm test
npm run build
npm start
```

See each service's own `README.md` for configuration and detailed documentation.
