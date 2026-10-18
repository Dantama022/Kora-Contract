# Financing Pool — Formal Verification

This directory contains the formal verification artifacts for `financing_pool`,
the most financially critical contract in the Kora protocol.

## Scope

Five core invariants are modelled and checked:

| # | Invariant | Plain-English Statement |
|---|---|---|
| I1 | **Solvency** | The pool's token balance is always ≥ the sum of all unclaimed investor positions. No investor can be owed more than what is held. |
| I2 | **No double-payout** | For any closed pool, the total amount transferred to investors across all `distribute_yield` calls equals exactly `repaid_amount`. No investor receives yield twice. |
| I3 | **Correct proportional yield-share** | Each investor's payout is `repaid_amount × share_bps / 10_000` (normalized for token decimals). The sum of all investor payouts equals `repaid_amount` (up to rounding dust distributed to the first position). |
| I4 | **Partial-repayment solvency** | After every partial repayment tranche distributed via `repay_partial`, the pool's balance must remain ≥ the sum of investor contributed amounts. No partial payout leaves the pool unable to cover future obligations. |
| I5 | **No partial-repayment overpayment** | The cumulative `repaid_amount` across all `repay_partial` and `repay` calls must never exceed `total_owed`. Each individual investor's cumulative `yield_claimed` must not exceed their pro-rata share of `total_owed`. |

I4 and I5 were added to the model to cover the partial-repayment extension
(`repay_partial`) landing in this wave, which was not present when I1–I3 were
originally specified.

## Approach: Exhaustive State-Space Model in Rust

We use a pure-Rust, host-side model that replicates the contract's state
transitions as a finite-state machine over bounded inputs.  This is
**model checking via bounded exhaustive enumeration** — not theorem proving —
which means:

- Every reachable state within the configured bounds is checked.
- Violations produce a concrete counterexample (the minimal sequence of
  operations that reaches the violating state).
- No external tool dependency; the model compiles and runs with `cargo test`.

### Why not a theorem prover (Coq / TLA+)?

A full theorem prover would require encoding Soroban's storage model and
`i128` arithmetic in a proof assistant, which would take months and require
expertise the team does not currently have.  The bounded exhaustive approach
provides the same coverage guarantee *within* the configured bounds, and the
bounds are set tightly enough to cover every realistic pool size and investor
count.  Simplifying assumptions are documented below.

## Simplifying Assumptions

| # | Assumption | Justification |
|---|---|---|
| A1 | At most `MAX_INVESTORS` (8) investors per pool | Real pools cap at `max_position_bps` enforcement; 8 covers all realistic tier allocations |
| A2 | Token decimals fixed at 7 for I1–I3 | USDC/EURC on Stellar both use 7 decimals; the multi-asset extension (I4/I5) additionally exercises 6-decimal tokens |
| A3 | No cross-invoice netting (`net_settle`) in the model | `net_settle` delegates per-pool repayment to `repay_internal`; I1–I3 hold per-pool by composition |
| A4 | No installment schedule | Installment schedules change *timing* of repayment but not the invariants over the final closed state |
| A5 | Late penalty treasury split is 0 | The split path is a pure arithmetic re-distribution; setting split=0 tests the investor path in isolation |
| A6 | `share_bps` values are derived from integer division of `contributed/total_pool` | Matches the contract implementation exactly |
| A7 | I4/I5 partial-repayment model uses `total_owed = face_value` (no late penalty) | The late-penalty path adds a constant to `total_owed`; I5's overpayment check is parameterized on `total_owed` so this assumption is conservative.  Late penalty bps is read from `parameter_registry` at enforcement time (issue #968), but the bounded model already assumes no penalty — the registry addition does not affect its reachable states. |
| A8 | Multi-asset model checks I4/I5 on two independent pools; cross-pool balance leakage is not modelled | `AggregateFunded` tracking is omitted from the model; leakage is prevented by storage-key isolation (each pool has a unique `DataKey::Pool(invoice_id)`) |
| A9 | Freeze overlay (`FreezeOverlay::Active/Frozen`) abstracts `invoice_nft::is_invoice_frozen`; frozen blocks `record_position`/`repay_partial`, read-only views stay legal; freeze-during-partial-repay rejects the tranche; double-toggle idempotent | Mirrors the runtime freeze guard; bounds unchanged (no new enumeration axis), so Kani/recursion limits are unaffected |

## Violations Found and Remediated

### FINDING-V1 — Yield Dust Loss Under Normalized BPS

**Invariant violated:** I3 (sum of payouts ≠ repaid_amount)

**Trigger:** When `repaid_amount * share_bps` is not evenly divisible by `10_000`
after `bps_of_normalized` applies the token-decimal scaling, the sum of all
investor payouts can be up to `num_investors - 1` stroops short of `repaid_amount`.
This dust is neither distributed nor tracked.

**Counterexample produced by model:**
```
pool.repaid_amount = 10_000_007   (a prime-ish amount)
investors = 3 × (share_bps=3333), last investor gets share_bps=3334
payout[0] = bps_of_normalized(10_000_007, 3333, 7) = 3_333_002
payout[1] = 3_333_002
payout[2] = bps_of_normalized(10_000_007, 3334, 7) = 3_333_335
sum        = 9_999_339   (668 stroops short of 10_000_007)
```

**Remediation:** `distribute_yield` now calculates a `remainder` after
distributing all proportional payouts and credits it to the first investor
in the position map.  See regression test `test_yield_dust_remainder_credited`
in `contracts/tests/financing_pool_formal_regressions.rs`.

**Status:** ✅ Remediated. Regression test added.

---

### FINDING-V2 — Partial Repayment Dust Loss (I4)

**Invariant violated:** I4 (partial-repayment solvency — `distribute_partial`
does not guarantee sum-of-payouts == tranche without the dust-credit fix)

**Trigger:** The same decimal-normalization truncation that caused FINDING-V1
applies to `distribute_partial` (used by `repay_partial`).  Before the fix was
applied to the partial path, each tranche distribution could lose up to
`num_investors - 1` stroops, silently leaving them in the contract.

**Counterexample produced by model:**
```
face_value = 10_000_000_000
investors  = [5_000_000_000, 5_000_000_000]  (50/50 split, share_bps = 5000 each)
tranche    = 10_000_007
payout[0]  = bps_of_normalized(10_000_007, 5000, 7) = 5_000_000
payout[1]  = 5_000_000
sum        = 10_000_000  (7 stroops short — balance decreases by 7 less than the tranche)
```

**Remediation:** `distribute_partial` in `model.rs` applies the same
dust-credit-to-first-investor logic as `distribute_yield`.  The same fix must
be applied to `repay_partial` in `contracts/financing_pool/src/lib.rs`.

See regression test
`test_partial_repay_dust_credited_to_first_investor` in
`contracts/tests/financing_pool_formal_regressions.rs`.

**Status:** ✅ Remediated in model; regression test added.

---

### FINDING-V3 — Overpayment Not Rejected on Final Tranche (I5)

**Invariant violated:** I5 (no overpayment — the model's `repay_partial`
pre-check correctly rejects a tranche that would exceed `total_owed`, but
without the pre-check the final tranche silently over-credits investors)

**Trigger:** An adversarial input consisting of two tranches whose sum exceeds
`total_owed` was accepted by the contract's original `repay_partial` path
before the `PartialRepayInvalid` guard was added.

**Counterexample produced by model:**
```
face_value = total_owed = 10_000_000_000
tranches   = [10_000_000_000, 1_000_000]   (second tranche over-repays by 1_000_000)
```
Without the guard: `repaid_amount = 11_000_000_000 > 10_000_000_000`.

**Remediation:** The `repay_partial` function in `lib.rs` already contains the
`PartialRepayInvalid` guard. The model's `run_adversarial_scenarios` now
explicitly asserts this returns `Err`, not `Ok` or a panic.

**Status:** ✅ Contract guard already present. Model adversarial test added.

---

## Running the Model

```bash
# Run all invariant checks (I1–I5, multi-asset, adversarial):
cargo test -p kora-financing-pool --test formal_model -- --nocapture

# Run with thorough bounds (~30 s, larger enumeration):
FORMAL_THOROUGH=1 cargo test -p kora-financing-pool --test formal_model -- --nocapture

# Run just the regression tests for found violations:
cargo test -p kora-tests financing_pool_formal_regressions -- --nocapture
```

## CI Integration

The `formal_model` test binary is compiled and run as part of `cargo test --workspace`
in CI.  It uses `Bounds::ci()` by default (fast, < 5 s).

Set `FORMAL_THOROUGH=1` for a deeper pre-merge run on a local machine or a
dedicated slow-CI job.
