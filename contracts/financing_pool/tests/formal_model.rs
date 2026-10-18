// contracts/financing_pool/tests/formal_model.rs
//!
//! Bounded exhaustive formal model tests for `financing_pool`.
//!
//! Run with:
//!   cargo test -p kora-financing-pool --test formal_model -- --nocapture
//!
//! Set `FORMAL_THOROUGH=1` in the environment for the larger bound set.

use kora_financing_pool::verification::invariants::{
    run_adversarial_scenarios, run_exhaustive_check, run_multi_asset_check,
    run_partial_repay_check, run_share_bps_sum_check, Bounds,
};

fn bounds() -> Bounds {
    if std::env::var("FORMAL_THOROUGH").is_ok() {
        Bounds::thorough()
    } else {
        Bounds::ci()
    }
}

// ── I1: Solvency ─────────────────────────────────────────────────────────────

#[test]
fn formal_i1_solvency_bounded_exhaustive() {
    let b = bounds();
    match run_exhaustive_check(&b) {
        Ok(n) => println!("I1 solvency: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (I1 solvency):\n{}", e),
    }
}

// ── I2: No double-payout ──────────────────────────────────────────────────────

#[test]
fn formal_i2_no_double_payout_bounded_exhaustive() {
    // I2 is checked inside run_exhaustive_check alongside I1 and I3.
    // This test is a named alias so CI output clearly labels the invariant.
    let b = bounds();
    match run_exhaustive_check(&b) {
        Ok(n) => println!("I2 no-double-payout: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (I2 no-double-payout):\n{}", e),
    }
}

// ── I3: Proportional yield-share ──────────────────────────────────────────────

#[test]
fn formal_i3_proportional_yield_bounded_exhaustive() {
    let b = bounds();
    match run_exhaustive_check(&b) {
        Ok(n) => println!("I3 proportional yield: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (I3 proportional yield):\n{}", e),
    }
}

// ── I4: Partial-repayment solvency ────────────────────────────────────────────

/// Verify I4: after every partial repayment tranche the pool balance is >=
/// sum of investor contributions (the pool holds at least the principal it owes).
///
/// This exercises the `repay_partial` path across a wide range of tranche
/// sequences, investor counts, and face values.
#[test]
fn formal_i4_partial_repayment_solvency_bounded_exhaustive() {
    let b = bounds();
    match run_partial_repay_check(&b) {
        Ok(n) => println!("I4 partial-repayment solvency: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (I4 partial-repayment solvency):\n{}", e),
    }
}

// ── I5: No partial-repayment overpayment ──────────────────────────────────────

/// Verify I5: cumulative `repaid_amount` never exceeds `total_owed`, and each
/// individual investor's `yield_claimed` never exceeds their pro-rata share.
///
/// This is checked inside `run_partial_repay_check` (both I4 and I5 are
/// asserted on every tranche).  This test alias gives CI output a distinct
/// label for the overpayment invariant.
#[test]
fn formal_i5_no_partial_repay_overpayment_bounded_exhaustive() {
    let b = bounds();
    match run_partial_repay_check(&b) {
        Ok(n) => println!("I5 no-overpayment: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (I5 no-overpayment):\n{}", e),
    }
}

// ── Multi-asset extension (I4/I5 across different token decimal precisions) ───

/// Verify that I4 and I5 hold independently for pools using different token
/// decimal configurations (7-decimal USDC and 6-decimal alternative).
///
/// Assumption A2 is relaxed here: the multi-asset check exercises the
/// `bps_of_normalized` formula with `decimals = 6` to confirm the partial
/// repayment invariants are not decimal-specific.
#[test]
fn formal_multi_asset_partial_repay_invariants() {
    let b = bounds();
    match run_multi_asset_check(&b) {
        Ok(n) => println!("multi-asset I4/I5: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (multi-asset):\n{}", e),
    }
}

// ── I-extra: Share-bps sum ────────────────────────────────────────────────────

#[test]
fn formal_share_bps_sum_never_exceeds_10000() {
    let b = bounds();
    match run_share_bps_sum_check(&b) {
        Ok(n) => println!("share_bps sum: {} splits checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (share_bps sum):\n{}", e),
    }
}

// ── Adversarial / boundary scenarios ─────────────────────────────────────────

/// Run the full set of hand-crafted adversarial scenarios, including:
/// - I1–I3 boundary cases (FINDING-V1 counterexample, dust pool, etc.)
/// - I4/I5 adversarial cases (micro-tranches, overpayment attempt, multi-asset)
#[test]
fn formal_adversarial_scenarios() {
    match run_adversarial_scenarios() {
        Ok(n) => println!("adversarial: {} scenarios checked — all passed", n),
        Err(e) => panic!("FORMAL VIOLATION (adversarial):\n{}", e),
    }
}

// ── Freeze overlay (invoice_nft::is_invoice_frozen) ───────────────────────────

/// Invariant: no position increment and no contribution increase while the
/// freeze overlay is active; unfreeze restores previously legal transitions.
/// Covers freeze-on-then-attempt-fund plus freeze-during-partial-repay.
#[test]
fn formal_freeze_overlay_blocks_fund_and_positions() {
    use kora_financing_pool::verification::model::{FreezeOverlay, PoolState};
    let mut s = PoolState::new(100_000_000);
    assert!(s.record_position(10_000_000).is_ok());
    let funded_before = s.pool.total_funded;
    let positions_before = s.positions.len();
    s.freeze();
    assert_eq!(s.freeze, FreezeOverlay::Frozen);
    // Freeze-on then attempt fund: must be rejected, state unchanged.
    assert!(s.record_position(10_000_000).is_err());
    assert!(s.repay_partial(1_000_000).is_err());
    assert_eq!(s.pool.total_funded, funded_before);
    assert_eq!(s.positions.len(), positions_before);
    // Read-only views stay legal while frozen.
    let _ = s.total_owed_view();
    // Double-freeze is idempotent.
    s.freeze();
    assert!(s.record_position(1).is_err());
    // Unfreeze restores previously legal transitions.
    s.unfreeze();
    assert!(s.record_position(10_000_000).is_ok());
    assert_eq!(s.positions.len(), positions_before + 1);
    println!("freeze overlay: fund blocked while frozen, restored after unfreeze");
}
