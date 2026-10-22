# kora-xtask

Workspace automation tools that run on the host (native, not wasm32).

## check-access-matrix (issue #917)

Verifies that every `pub fn` in every deployed contract crate has a
corresponding entry in `docs/ACCESS_MATRIX.md` and that the authorization
implementation matches the documented role.

```sh
# Run the checker
cargo run -p kora-xtask --bin check-access-matrix

# Run with verbose output (shows each function checked)
cargo run -p kora-xtask --bin check-access-matrix -- --verbose

# Run for a single contract
cargo run -p kora-xtask --bin check-access-matrix -- --contract secondary_market
```

Exit code `0` = all functions are documented and match. Exit code `1` = drift
or undocumented entrypoints found.

### Covered contracts (as of issue #917)

| Contract | Source |
|----------|--------|
| `access_control` | `contracts/access_control/src/lib.rs` |
| `risk_registry` | `contracts/risk_registry/src/lib.rs` |
| `invoice_nft` | `contracts/invoice_nft/src/lib.rs` |
| `marketplace` | `contracts/marketplace/src/lib.rs` |
| `financing_pool` | `contracts/financing_pool/src/lib.rs` |
| `price_oracle` | `contracts/price_oracle/src/lib.rs` |
| `dispute_resolution` | `contracts/dispute_resolution/src/lib.rs` |
| `treasury` | `contracts/treasury/src/lib.rs` |
| `secondary_market` | `contracts/secondary_market/src/lib.rs` |
| `fractionalizer` | `contracts/fractionalizer/src/lib.rs` |
| `dispute_escrow` | `contracts/dispute_escrow/src/lib.rs` |
| `governance_voting` | `contracts/governance_voting/src/lib.rs` |
| `referral_rewards` | `contracts/referral_rewards/src/lib.rs` |

### How to add a new contract

1. Add the contract's directory name to the `contract_dirs` array in
   `contracts/xtask/src/bin/check_access_matrix.rs`.
2. Add matrix entries in `load_access_matrix()` for every `pub fn` in the
   new contract's `#[contractimpl]` block.
3. Add the corresponding rows to `docs/ACCESS_MATRIX.md`.
4. Run `cargo run -p kora-xtask --bin check-access-matrix` and confirm exit 0.

### How to add a new function to an existing contract

1. Add the row to `load_access_matrix()` in `check_access_matrix.rs`:
   ```rust
   matrix.insert(("my_contract".into(), "my_new_fn".into()), Authorization::Admin);
   ```
2. Add the corresponding row to `docs/ACCESS_MATRIX.md`.
3. Run the checker to confirm exit 0.

**Undocumented new entrypoint = CI failure** — the `access-matrix-check`
job in `.github/workflows/ci.yml` blocks the PR.

## check-error-variants

Verifies that every `KoraError::Variant` referenced across contract crates
is declared on the `KoraError` enum in `contracts/shared/src/errors.rs`.

```sh
cargo run -p kora-xtask --bin check-error-variants
```

## extract-spec (issue #937)

Extracts ABI and contract specifications (`contractspecv0`) from compiled Soroban WASM files using `soroban-spec` and `stellar-xdr`.

```sh
# Extract spec for a single contract as deterministic JSON:
cargo run -p kora-xtask --bin extract-spec -- target/wasm32-unknown-unknown/release/kora_invoice_nft.wasm

# Extract to a specified file:
cargo run -p kora-xtask --bin extract-spec -- target/wasm32-unknown-unknown/release/kora_invoice_nft.wasm --out specs/invoice_nft.spec.json

# Batch extract all contracts in a directory into interface text format:
cargo run -p kora-xtask --bin extract-spec -- --dir target/wasm32-unknown-unknown/release --out specs/ --format text
```

### Options:
- `[WASM_FILES...]`: One or more paths to compiled `.wasm` files.
- `--dir <DIRECTORY>`: Directory containing `.wasm` files to extract in batch.
- `--out <PATH>`: Output destination (file or directory). Defaults to `stdout`.
- `--format <json|text>`: Output format (`json` by default, or `text`/`interface` for interface compatibility checks).

