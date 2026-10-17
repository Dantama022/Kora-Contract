# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅ Yes     |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

### Submission

**Email:** security@kora.finance (PGP key available at [kora.finance/.well-known/pgp-key.txt](https://kora.finance/.well-known/pgp-key.txt))

**GitHub Security Advisory:** [Create private advisory](https://github.com/kora-finance/contracts/security/advisories/new)

**Required Information:**
- Description of the vulnerability
- Steps to reproduce (proof-of-concept code preferred)
- Potential impact and severity assessment
- Affected contract(s) and function(s)
- Suggested fix (optional)

### Response SLA

| Severity | Acknowledgment | Patch Timeline |
|----------|---------------|----------------|
| **Critical** | 4 hours | 48 hours |
| **High** | 24 hours | 7 days |
| **Medium** | 72 hours | 30 days |
| **Low** | 1 week | Best effort |

### Bug Bounty Program

We offer rewards for valid vulnerability reports:
- **Critical:** $10,000 - $50,000 USD
- **High:** $2,000 - $10,000 USD
- **Medium:** $500 - $2,000 USD
- **Low:** $100 - $500 USD

See **[docs/BUG_BOUNTY_OPERATIONS.md](docs/BUG_BOUNTY_OPERATIONS.md)** for full program details, scope, and responsible disclosure policy.

### Additional Resources

- **Security Model:** [docs/SECURITY.md](docs/SECURITY.md)
- **Access Control Matrix:** [docs/ACCESS_MATRIX.md](docs/ACCESS_MATRIX.md) — every public entrypoint across all contracts mapped to its required role; enforced in CI by `check_access_matrix`, which parses the matrix as the single source of truth
- **Oracle Defense Testing:** [docs/ORACLE_ADVERSARIAL_TESTING.md](docs/ORACLE_ADVERSARIAL_TESTING.md)
- **Loop Bounds Audit:** [docs/LOOP_BOUNDS_AUDIT.md](docs/LOOP_BOUNDS_AUDIT.md)
