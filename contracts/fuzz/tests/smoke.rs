//! Deterministic smoke harness: the stable-Rust CI check for the fuzz targets.
//!
//! For each contract it replays every file in `corpus/<contract>/`, then runs
//! `FUZZ_ITERS` (default 10_000) iterations of a seeded RNG through the target's
//! `run`. A panic inside any `run` fails the test with the reproducing input as
//! a hex string, which can be dropped into `corpus/` or fed to `cargo fuzz`.
//!
//! These tests are `#[ignore]`d so `cargo test --workspace` stays fast; the
//! `fuzz` CI job and `make fuzz` run them with `-- --ignored`.

use soroban_sdk::testutils::arbitrary::fuzz_catch_panic;

use rand::{RngCore, SeedableRng};

fn iters() -> usize {
    return std::env::var("FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    return s;
}

fn check_once(name: &str, f: fn(&[u8]), data: &[u8], ctx: &str) {
    let owned = data.to_vec();
    let call = move || f(&owned);
    if fuzz_catch_panic(call).is_err() {
        panic!(
            "FUZZ FINDING in target `{name}` ({ctx})\n  \
             reproduce: echo -n {hex} | xxd -r -p > repro.bin && cargo fuzz run fuzz_{name} repro.bin\n  \
             input (hex): {hex}",
            hex = hex(data),
        );
    }
}

fn replay_corpus(name: &str, f: fn(&[u8])) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(name);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let data = std::fs::read(&path).expect("read corpus file");
            check_once(name, f, &data, &format!("corpus:{}", path.display()));
        }
    }
}

fn run_target(name: &str, f: fn(&[u8]), seed: u64) {
    replay_corpus(name, f);

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut buf = [0u8; 512];
    let n = iters();
    for i in 0..n {
        let len = 16 + (rng.next_u32() as usize % (buf.len() - 16));
        rng.fill_bytes(&mut buf[..len]);
        check_once(name, f, &buf[..len], &format!("seed={seed} iter={i}/{n}"));
    }
}

macro_rules! smoke {
    ($fn:ident, $name:literal, $target:path, $seed:literal) => {
        #[test]
        #[ignore = "fuzz smoke; run via `make fuzz` or `cargo test -p kora-fuzz -- --ignored`"]
        fn $fn() {
            run_target($name, $target, $seed);
        }
    };
}

smoke!(fuzz_access_control, "access_control", kora_fuzz::targets::access_control::run, 0xAC01);
smoke!(fuzz_invoice_nft, "invoice_nft", kora_fuzz::targets::invoice_nft::run, 0x0F72);
smoke!(fuzz_marketplace, "marketplace", kora_fuzz::targets::marketplace::run, 0x3A17);
// Dedicated harness for fee-calculation and tiered-fee invariants (INV-FEE-1..7).
// Uses a separate corpus directory (`corpus/marketplace_fee/`) and seed so its
// input space is explored independently from the general marketplace lifecycle harness.
smoke!(fuzz_marketplace_fee, "marketplace_fee", kora_fuzz::targets::marketplace_fee::run, 0xFE3A);
smoke!(fuzz_financing_pool, "financing_pool", kora_fuzz::targets::financing_pool::run, 0x9001);
smoke!(fuzz_treasury, "treasury", kora_fuzz::targets::treasury::run, 0x7EA5);
smoke!(fuzz_risk_registry, "risk_registry", kora_fuzz::targets::risk_registry::run, 0x815C);
smoke!(fuzz_price_oracle, "price_oracle", kora_fuzz::targets::price_oracle::run, 0x0AC1);
smoke!(fuzz_dispute_resolution, "dispute_resolution", kora_fuzz::targets::dispute_resolution::run, 0xD159);
smoke!(fuzz_secondary_market_settlement, "secondary_market_settlement", kora_fuzz::targets::secondary_market::run, 0x5EC0);

#[test]
fn secondary_market_settlement_ci_corpus_replay() {
    replay_corpus("secondary_market_settlement", kora_fuzz::targets::secondary_market::run);
}

#[test]
fn secondary_market_settlement_ci_bounded_random_walk() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5EC0_C1);
    let mut buf = [0u8; 256];
    for i in 0..64 {
        rng.fill_bytes(&mut buf);
        check_once(
            "secondary_market_settlement",
            kora_fuzz::targets::secondary_market::run,
            &buf,
            &format!("ci_secondary_market_random_walk iter={i}/64"),
        );
    }
}

#[test]
fn dispute_invariants_ci_corpus_replay() {
    replay_corpus("dispute_resolution", kora_fuzz::targets::dispute_resolution::run);
}

#[test]
fn dispute_invariants_ci_bounded_random_walk() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xD159_C1);
    let mut buf = [0u8; 4 * kora_fuzz::MAX_OPS];
    for i in 0..32 {
        rng.fill_bytes(&mut buf);
        check_once("dispute_resolution", kora_fuzz::targets::dispute_resolution::run,
            &buf, &format!("ci_dispute_random_walk iter={i}/32"));
    }
}

// ── CI-bounded fee invariant tests ────────────────────────────────────────────
//
// These are NOT ignored — they run on every `cargo test` as part of the normal
// CI pass.  They are bounded to `CI_FEE_ITERS` iterations (default 500) so they
// complete in < 1 s.  They replicate the INV-FEE-1..7 seed-corpus tests from
// `marketplace_fee::seeds` but also drive the `run` entry-point with a short
// random walk, ensuring the CI gate is not purely deterministic.
//
// The full smoke harness (`fuzz_marketplace_fee` above, `--ignored`) runs
// 10 000 iterations; this is the quick sanity gate.

fn ci_fee_iters() -> usize {
    std::env::var("CI_FEE_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
}

#[test]
fn fee_invariants_ci_bounded_random_walk() {
    use rand::{RngCore, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xFEE_C1_B0);
    let mut buf = [0u8; 256];
    let n = ci_fee_iters();
    for i in 0..n {
        let len = 8 + (rng.next_u32() as usize % (buf.len() - 8));
        rng.fill_bytes(&mut buf[..len]);
        check_once(
            "marketplace_fee",
            kora_fuzz::targets::marketplace_fee::run,
            &buf[..len],
            &format!("ci_fee_random_walk iter={i}/{n}"),
        );
    }
}

#[test]
fn fee_invariants_ci_corpus_replay() {
    // Replay every seed in the marketplace_fee corpus without the --ignored gate.
    replay_corpus("marketplace_fee", kora_fuzz::targets::marketplace_fee::run);
}

#[test]
fn fee_invariants_ci_seed_cases() {
    // Run the hand-crafted seed cases defined in the harness itself.
    kora_fuzz::targets::marketplace_fee::assert_fee_seeds_representative();
    kora_fuzz::targets::marketplace_fee::assert_fee_seeds_tier_breakpoints();
}

// ── Issue #918: secondary_market buy_position CEI ordering ───────────────────
//
// Full fuzz smoke target (--ignored; driven by CI `dispute-fuzz-smoke`-style job
// or `make fuzz`).  Asserts INV-SM-1 through INV-SM-4 across all op sequences.
smoke!(
    fuzz_secondary_market,
    "secondary_market",
    kora_fuzz::targets::secondary_market::run,
    0x5EC0
);

/// CI-bounded (non-ignored) smoke test for secondary_market CEI invariants.
///
/// Runs 200 iterations with a seeded RNG so it completes in < 1 s on CI.
/// The full smoke harness (`fuzz_secondary_market`, --ignored) runs 10_000.
#[test]
fn secondary_market_cei_ci_bounded() {
    use rand::{RngCore, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x5EC0_CEI1);
    let mut buf = [0u8; 256];
    let n = std::env::var("CI_SM_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200usize);
    for i in 0..n {
        let len = 8 + (rng.next_u32() as usize % (buf.len() - 8));
        rng.fill_bytes(&mut buf[..len]);
        check_once(
            "secondary_market",
            kora_fuzz::targets::secondary_market::run,
            &buf[..len],
            &format!("ci_sm_cei iter={i}/{n}"),
        );
    }
}

/// Replay corpus entries for secondary_market (no --ignored gate).
#[test]
fn secondary_market_cei_corpus_replay() {
    replay_corpus("secondary_market", kora_fuzz::targets::secondary_market::run);
}
