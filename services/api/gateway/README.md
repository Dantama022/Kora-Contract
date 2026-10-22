# API Gateway

**Issue #753, Issue #932 — REST/GraphQL API Gateway for Kora Protocol Data**

High-performance gateway over the indexed protocol data (invoices, listings, positions, and risk scores). Serves frontend applications and third-party integrators with filtering, cursor-based pagination, and deterministic ordering.

Hardened with credential-based authentication, contract-scoped rate limiting, and zero secret leakage.

## Architecture

```
                      Client Request
                            │
               ┌────────────┴────────────┐
               │  Route & Method Check   │
               └────────────┬────────────┘
                            │
         ┌──────────────────┴──────────────────┐
         │                                     │
   [Public Read-Only]                   [Mutating / Admin]
   GET /health, /invoices, ...          POST /contracts/:id/..., /admin/...
         │                                     │
         │                              ┌──────┴──────┐
         │                              │ Authenticate│ (API Key / JWT)
         │                              └──────┬──────┘
         │                                     │ (401 on missing/invalid)
         │                                     │ (403 on insufficient scope)
         ▼                                     ▼
   ┌───────────┐                         ┌───────────┐
   │Rate Limit │                         │Rate Limit │
   │ (API-key) │                         │(Key+Scope)│
   └─────┬─────┘                         └─────┬─────┘
         │ (429 + Retry-After)                 │ (429 + Retry-After)
         ▼                                     ▼
   Serve Indexed Data                    Execute Route Handler
```

## Security & Authentication

### Mutating & Protected Routes
All state-modifying requests (`POST`, `PUT`, `DELETE`, `PATCH`) require valid authentication via:
1. **API Key**: Passed via `X-Api-Key: <key>` or `Authorization: ApiKey <key>`.
2. **JWT**: Passed via `Authorization: Bearer <token>`.
   - Signed with HMAC-SHA256 (`HS256`) using `JWT_SECRET`.
   - Expiration (`exp`) and not-before (`nbf`) claims are strictly enforced with configurable clock-skew tolerance.
   - When configured, `iss` (issuer) and `aud` (audience) claims are validated against `JWT_ISSUER` and `JWT_AUDIENCE`.
   - Caller identity is derived exclusively from validated token claims (`sub`, `userId`, `apiKey`), never trusted from spoofable client headers.

Unauthenticated callers receive `401 Unauthorized`. Authenticated callers without admin scope attempting `/admin/*` operations receive `403 Forbidden`.

### Public Read-Only Routes
Public access is preserved for:
- `GET /health` — Liveness probe (unauthenticated, 200 `{ status: "ok" }`).
- `GET /invoices`
- `GET /listings`
- `GET /positions`
- `GET /risk-scores`

### Secret Redaction & Safe Diagnostics
- API keys, JWT signatures, and authorization headers are automatically redacted in server logs (replaced with `[REDACTED]` or partial masks).
- Diagnostic context (such as failure reasons `signature_mismatch`, `token_expired`, `unknown_api_key`) is logged internally without exposing raw secrets.
- Error responses return clean error summaries without leaking token signatures, payloads, or internal error details.

## Rate Limiting

### Access Tiers (Per-Key)
Token-bucket per API key:
| Tier | Request Budget | Window |
|---|---|---|
| `free` | 60 req | 60 s |
| `standard` | 600 req | 60 s |
| `enterprise` | 6000 req | 60 s |

Unknown or unauthenticated read requests default to `free` tier.

### Contract-Scoped Rate Limiting
Mutating and contract-sensitive routes enforce **dual-bucket rate limits**:
- An API key budget (protecting global service capacity).
- A contract-scoped budget (`contract:<key>:<contractId>`), preventing high-frequency bursts on one contract from degrading other protocol workflows.
- Contract identifiers are normalized (uppercase, trimmed, validated against `^[A-Z0-9_-]{1,64}$`) before bucket keys are derived.
- Default limit: 30 requests per minute per contract (custom limits can be registered via `registerContractLimit`).
- If a rate limit is exceeded, the server returns HTTP `429 Too Many Requests` with a `Retry-After: <seconds>` header.

## Environment Variables

| Variable | Description | Example |
|---|---|---|
| `JWT_SECRET` | Secret key for verifying HS256 JWT tokens | `super-secret-hmac-key` |
| `JWT_ISSUER` | Expected JWT issuer claim (`iss`) | `kora-auth-service` |
| `JWT_AUDIENCE` | Expected JWT audience claim (`aud`) | `kora-api-gateway` |
| `GATEWAY_API_KEYS` | Comma-separated list of `key:tier` | `key1:free,key2:standard,key3:enterprise` |
| `GATEWAY_ADMIN_KEYS` | Comma-separated list of admin API keys | `admin-key-secret` |
| `PORT` | Gateway HTTP listen port | `3001` |

## Usage Examples

### Starting HTTP Gateway
```ts
import { ApiGateway, RateLimiter, Authenticator, createMemoryStore } from "@kora/api-gateway";

const store = createMemoryStore();
const limiter = new RateLimiter();
const authenticator = new Authenticator({
  jwtSecret: process.env.JWT_SECRET,
  jwtIssuer: "kora.finance",
});

const gateway = new ApiGateway(store, limiter, authenticator);
const server = gateway.createHttpServer();
server.listen(3001, () => {
  console.log("API Gateway listening on port 3001");
});
```

### Querying Indexed Data
```bash
# Public query (with cursor pagination)
curl "http://localhost:3001/invoices?status=Listed&limit=20"

# Query with API key
curl -H "X-Api-Key: partner-key" "http://localhost:3001/listings?seller=SME_A"
```

### Mutating Route with Authentication
```bash
# Protected mutation using JWT
curl -X POST \
  -H "Authorization: Bearer <jwt_token>" \
  -H "Content-Type: application/json" \
  -d '{"action":"batch_invoke"}' \
  "http://localhost:3001/contracts/CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF/invoke"

# Over-budget response (429)
# HTTP/1.1 429 Too Many Requests
# Retry-After: 45
# {"error": "Too Many Requests", "retryAfter": 45}
```

## Testing

```bash
npm install
npm test
```
