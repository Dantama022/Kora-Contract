# Debtor Attestation Contract

## Overview

The `debtor_attestation` contract provides a lightweight on-chain mechanism for the named debtor on an invoice to cryptographically confirm the invoice's authenticity and amount before it becomes eligible for marketplace listing.

This addresses the top risk in trade finance of fraudulent or fabricated invoices. An on-chain debtor confirmation step - even via a delegated signature or oracle relay - meaningfully reduces fraud risk and increases investor confidence versus SME self-attestation alone.

## Architecture

The contract maintains a registry of attestation records keyed by invoice ID. Each record captures:

- **invoice_id** - the invoice being attested
- **debtor** - the debtor address that confirmed the invoice
- **amount** - the confirmed face amount
- **attested_at** - ledger timestamp of confirmation
- **expires_at** - timestamp after which the attestation is stale
- **status** - `Pending | Confirmed | Rejected | Expired`
- **relay** - optional relay address if attested via relay

## Key Functions

### Administration

- **initialize(env, admin, max_attestation_age)** - one-time setup. `admin` is the governance address that can whitelist relays and tune the max attestation age. Default max age is 30 days.
- **set_max_attestation_age(env, age)** - admin-only. Updates the maximum allowed age of an attestation.
- **max_attestation_age(env)** - reads the configured max age.
- **add_relay(env, relay)** - admin-only. Whitelists a relay address.
- **remove_relay(env, relay)** - admin-only. Removes a relay from the whitelist.
- **is_relay(env, relay)** - returns whether an address is a whitelisted relay.

### Attestation

- **attest(env, debtor, invoice_id, amount, expires_at, nonce, signature)** - direct debtor attestation. The debtor signs a message committing to the invoice ID, amount, expiry, and nonce. The contract verifies the signature against the debtor address and records the attestation.
- **attest_via_relay(env, relay, debtor, invoice_id, amount, expires_at, nonce, relay_sig)** - relay-based attestation for debtors without direct wallets. Only whitelisted relays can submit. The relay signs a message committing to the debtor address and invoice details.
- **reject(env, caller, invoice_id)** - the debtor or a whitelisted relay can reject an attestation (e.g. if the debtor disputes the invoice).

### Read Only

- **attestation_status(env, invoice_id)** - returns the current effective status, accounting for expiry. This is the integration point consumed by the marketplace at listing time.
- **is_confirmed(env, invoice_id, expected_amount)** - returns true only if the invoice has a valid, non-expired confirmation and the recorded amount matches the expected amount.
- **get_record(env, invoice_id)** - returns the full attestation record for an invoice.
- **nonce_of(env, debtor)** - returns the next expected nonce for a debtor.

## Security Considerations

### Replay Protection

Each attestation includes a monotonically increasing nonce per debtor. The contract rejects any signature with a nonce that does not match the expected value, preventing replay of old attestations.

### Expiry Handling

Attestations carry an explicit expiry timestamp. The contract rejects attestations whose expiry has already passed at submission time, and `attestation_status` transitions to `Expired` once the expiry timestamp is reached. Stale confirmations do not count indefinitely.

### Relay Restrictions

Only addresses explicitly whitelisted by the admin can submit relay attestations. The contract verifies the relay signature against the relay address and rejects unwhitelisted relays.

## Marketplace Integration

The marketplace consults `attestation_status ` at listing time. Listing requirements are configurable per risk tier:

- **High-risk tier** - attestation is required; invoices must report `Confirmed` status.
- **Lower tiers** - attestation is optional; invoices may be listed without confirmation.

The invoice NFT exposes a read-only status check that the marketplace consumes. The attestation contract address is configured on the invoice NFT contract during initialization.

## Invoice Metadata Pipeline (Issues #1001, #1002)

The debtor attestation verifies *who* confirmed the invoice; the metadata
pipeline guarantees *what* was confirmed never silently changed. The document
commitment chain is:

1. **Upload** — the SME submits invoice document bytes in the web submission
   wizard. The wizard computes SHA-256 over the exact bytes with Web Crypto
   and rejects empty files before this step.
2. **Pin** — bytes are uploaded to `services/ipfs-pinning`
   (`apps/web/src/services/ipfsPinningClient.ts`, configured via
   `VITE_IPFS_PINNING_URL`), which hash-verifies the claimed digest **before**
   pinning, pins across ≥2 providers, and persists a pin-status record. The
   returned CID is the content-derived digest.
3. **Verify** — the wizard re-fetches the pinned bytes
   (`GET /pins/:cid/content`) and re-computes the SHA-256, mirroring the
   pinning service's `verifier.ts` client-side. Gateway or pinning-service
   content swaps fail **closed**: the mint step stays disabled while the
   computed hash differs from the hash the user confirmed on the review step.
4. **Commit** — the validated 32-byte SHA-256 is passed to the SDK mint
   channel as `metadata_hash: Bytes` (`invoice_nft::commit_metadata_hash`
   for the freshly minted invoice; raw digest bytes, hex displayed to the
   user). This is the same on-chain commitment audited by
   `flag_metadata_mismatch` / `resolve_metadata_dispute`.

The debtor attestation record therefore attests a borrower whose document is
anchored by an immutable on-chain `metadata_hash` — a mismatch between the
pinned IPFS document and the committed hash freezes the invoice for dispute
resolution.

## Testing

The contract is tested with a minimum of 90% coverage, including:

-1. Direct-signature attestation (debtor signs directly)
2. Relay attestation (whitelisted relay signs on behalf of debtor)
3. Expired attestation rejection
4. Listing block without attestation on required tiers
5. Replay protection (nonce enforcement)
6. Unwhitelisted relay rejection
7. Rejection flow
8. Admin operations (relay whitelisting, max age updates)

## Events

- `attest` - emitted when an attestation is recorded (includes invoice ID, debtor, amount, expiry)
- `reject` - emitted when an attestation is rejected
- `relay_add` - emitted when a relay is whitelisted
- `relay_remove` - emitted when a relay is removed
- `max_age` - emitted when the max attestation age is updated

## Error Codes

| Code | Name | Description |
|------|------|-------------|
| 1 | NotInitialized | Contract has not been initialized |
| 2 | AlreadyInitialized | Contract has already been initialized |
| 3 | NotAuthorized | Caller is not authorized for this operation |
| 4 | RelayNotWhitelisted | Relay address is not whitelisted |
| 5 | InvalidSignature | Signature verification failed |
| 6 | InvalidAmount | Amount does not match |
| 7 | AttestationExpired | Attestation has expired |
| 8 | AttestationNotFound | No attestation record found |
| 9 | AttestationRejected | Attestation was rejected |
| 10 | InvalidNonce | Nonce does not match expected value |
| 11 | InvalidExpiry | Expiry timestamp is invalid |

## Out of Scope

Building the off-chain debtor communication/SMS relay itself is tracked under Backend. This contract only provides the on-chain attestation primitives.

## Deployment & Configuration

`scripts/deploy.sh` supports deploying and initializing `debtor_attestation` via the following environment variables:

- `DEBTOR_ATTESTATION_ADMIN` — Admin address for the attestation contract (default: `$ADMIN`)
- `DEBTOR_ATTESTATION_MAX_AGE` — Maximum attestation age in seconds (default: `2592000` / 30 days)
- `DEBTOR_ATTESTATION_RELAYS` — Comma-separated list of initial relay addresses to whitelist (optional)
- `AUTO_WIRE_ATTESTATION` — Set to `true` to auto-wire marketplace and invoice_nft settings post-deploy (default: `false`)
