# Disputed repayment escrow (#747)

`dispute_escrow` holds a disputed payment in the pool's token until a single
adjudicator decides whether it should repay the invoice or return to its SME
payer. The admin can be a governance multisig contract address. It is separate
from the existing `dispute_resolution` contract for challenges to defaults.

## Deployment

1. Deploy and initialize `dispute_resolution`, then initialize `financing_pool`
   with its address. The resolution contract handles default challenges and is
   separate from repayment escrow.
2. Deploy and initialize `dispute_escrow` with the financing pool address.
3. Call `financing_pool.set_dispute_escrow(admin, escrow_address)` once,
   configure `financing_pool.set_marketplace(admin, marketplace_address)`, and
   apply any nonzero marketplace referral split with
   `marketplace.set_referrer_split_bps(admin, bps)`.
4. Make sure the financing pool is authorized to mark the invoice NFT repaid
   and the escrow contract has sufficient ledger lifetime for open disputes.

The pool's escrow address is set once by its admin. The callback arguments
are `reserve_escrow(escrow, invoice_id, amount)` and
`settle_escrow(escrow, invoice_id, amount, pay_pool)`; both authenticate the
configured escrow address and validate the reservation. `escrow_terms` and
`has_position` supply the custody contract's read interface. No additional
environment variables are required by this integration.

## Flow

`open_dispute(challenger, payer, invoice_id, amount)` requires signatures from
the challenger and SME payer. The challenger must be the SME or an investor
with a recorded position in that invoice. The amount must be positive and no
greater than the pool's remaining debt, in the pool token's smallest units.
The escrow reserves the invoice in the pool and transfers the payment directly
from the SME into escrow. It does **not** increment `repaid_amount`, protocol
repayment statistics, or investor balances. Normal `repay`, `repay_partial`,
`net_settle`, and `mark_default` are blocked while the reservation is open.

Before seven days have elapsed, the configured admin calls
`resolve_dispute(resolver, invoice_id, pay_pool)`. `pay_pool=true` transfers
the payment to the financing pool and calls its authenticated
`settle_escrow` hook; that hook credits repayment and, when the invoice is
fully settled, distributes yield and marks the NFT repaid. Partial escrow
payments remain in the pool until the remaining debt is repaid through
`repay`; investor balances stay unchanged until final distribution.
`pay_pool=false` returns the token to the SME payer and clears the reservation
without crediting repayment.
After seven days, anyone can call `resolve_timeout(invoice_id)` to refund the
SME and unblock normal settlement. An invoice may enter this escrow **once**:
neither resolution nor timeout permits a new dispute for the same invoice.

The escrow and pool callbacks execute in one transaction. A failed transfer
or settlement rolls back the escrow adjudication; its custody record remains open.
Inspect `get_dispute(invoice_id)` and the pool's `get_pool(invoice_id)` to
verify the outcome. This path applies the exact deposited pool-token amount.
Opening escrow is restricted to funded, unfrozen pools with no prior repayment,
installment schedule, or early-settlement offer, and a zero late-penalty rate.
The invoice currency must match the token metadata symbol, the token must use
seven decimals, and remaining debt must be below the high-value repayment
approval threshold (10,000,000,000 base units). Other debts use the existing
repayment/approval paths. A freeze after opening prevents pool-directed
settlement; refund remains available. Partial escrow credits cannot enter
`repay_partial`, `net_settle`, new position recording, installment schedules,
or early-settlement offers. This prevents mixed-path payouts and duplicate
repayment statistics while held funds await normal repayment or default.

## Validation

### Workspace and release size gate (#911)

`kora-dispute-escrow` is a workspace member. `make build` explicitly builds
the deployable contract packages, including this crate, and produces
`target/wasm32-unknown-unknown/release/kora_dispute_escrow.wasm`.
`make build-optimized` optimizes it alongside the other deployable contracts.
The WASM size workflow requires the release artifact and applies the
`dispute_escrow` budget in `wasm-budgets.json` (122,880 bytes); an overage
fails CI under the existing size-increase approval policy. Release hash
recording includes the same artifact. See [performance budgets](PERFORMANCE.md)
for the provisional-cap rationale; this onboarding did not run builds or
measure the binary.

`scripts/deploy.sh` deploys `dispute_resolution` alongside the financing pool
and deploys repayment escrow by default (`DEPLOY_DISPUTE_ESCROW=true`). It
initializes the dependencies in order, configures the pool's marketplace and
escrow addresses, and records deployed contract addresses and WASM hashes in the
manifest. Set `DEPLOY_DISPUTE_ESCROW=false` only when intentionally omitting the
repayment custody flow.

### State-machine fuzzing (#919)

See [the dispute fuzz model and coverage gaps](dispute-fuzz.md) and the
[fuzz target inventory](../contracts/fuzz/README.md). Resolution of defaults
and repayment custody are separate APIs; a mock cannot establish production
pool/token integration safety.

The escrow crate has tests for custody before resolution, both adjudication
routes, repeat-dispute rejection, and the timeout boundary. Run
`cargo test -p kora-dispute-escrow` and the financing pool suite in a working
Rust workspace, then measure coverage against the repository's 90% gate.
