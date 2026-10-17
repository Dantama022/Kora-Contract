# kora-fuzz

Structured fuzzing for Kora contract entry points and dispute state transitions.
Feeds randomised-but-structurally-valid call sequences through the generated
Soroban clients, hunting for panics, unchecked overflows, and unexpected
`Ok` on invalid input.

## Layout

| Path | What |
| --- | --- |
| `src/harness.rs` | Deploys and wires the full protocol into a fresh `Env` per input |
| `src/gen.rs` | Turns raw fuzz bytes into Soroban values, skewed toward boundaries |
| `src/targets/<contract>.rs` | `run(&[u8])` per contract: decode an op sequence, apply via `try_*` |
| `tests/smoke.rs` | Deterministic stable-Rust harness (the CI check) |
| `fuzz/` | `cargo-fuzz` / libFuzzer targets (nightly), calling the same `run` fns |
| `corpus/<contract>/` | Seed inputs (fixture-derived constants + edge byte patterns) |

## Running

### Smoke harness (stable Rust, CI)

```
make fuzz                    # 10_000 seeded iterations per contract + seed corpus
make fuzz FUZZ_ITERS=100000  # more iterations
cargo test -p kora-fuzz --test smoke -- --ignored --nocapture
```

The smoke tests are `#[ignore]`d so `cargo test --workspace` stays fast. CI runs
them through the `fuzz` job.

### Deep fuzzing (nightly + cargo-fuzz)

```
rustup toolchain install nightly
cargo install cargo-fuzz

make fuzz-deep FUZZ_TARGET=marketplace FUZZ_RUNS=5000000
# or directly:
cd contracts/fuzz/fuzz && cargo +nightly fuzz run fuzz_marketplace
```

Targets: `fuzz_access_control`, `fuzz_invoice_nft`, `fuzz_marketplace`,
`fuzz_financing_pool`, `fuzz_treasury`, `fuzz_risk_registry`,
`fuzz_price_oracle`, `fuzz_marketplace_fee`, `fuzz_dispute_resolution`,
`fuzz_secondary_market_settlement`.

### Dispute state transitions (#919)

`fuzz_dispute_resolution` compares every persisted dispute field against an
independent state model after every operation, including failed calls. It uses
a registered access-control mock with signer and pause controls, plus the real
dispute escrow and token contracts with a mock pool to check balances and
single release. The resolution and escrow APIs remain independent.

Each opcode occupies four bytes: operation (modulo 10), invoice (modulo 4),
actor/auth selector, argument. Actors 0–3 are admin, governance signer/investor,
payer/challenger, and outsider; actor bit 7 removes authentication. Operations
0–9 are open, evidence, resolve, advance time, configure fixture, open escrow,
adjudicate escrow, timeout refund, repeat initialization, and read. Up to 24
operations run per input. See [coverage and seed details](../../docs/dispute-fuzz.md).

The default test suite replays the dispute corpus and 32 deterministic random
walks; the ignored smoke entry runs `FUZZ_ITERS` iterations. For deep fuzzing,
pass the same seed directory explicitly:

```sh
cd contracts/fuzz/fuzz
cargo +nightly fuzz run fuzz_dispute_resolution ../corpus/dispute_resolution
```

## When a finding is reported

A panic inside any `run` is a bug: contracts must reject malformed input with a
returned `Err`, never a panic (host trap, `unwrap` on absent state, arithmetic
overflow under debug assertions).

1. The smoke harness prints the reproducing input as hex and a `cargo fuzz run`
   line. libFuzzer writes the input to `fuzz/artifacts/`.
2. Save the input under `corpus/<contract>/` so it becomes a regression seed.
3. File an issue with the contract, the decoded op sequence (`RUST_LOG` / add a
   `dbg!` in the target), and the hex input.
4. Fix the contract, confirm `make fuzz` is clean, keep the seed.

## Baseline

Built against commit `67a44fa` (the last state where `main` compiles cleanly;
later merges left `main` corrupted). Entry-point signatures and the harness
wiring track that commit and need a pass once `main` is repaired.
