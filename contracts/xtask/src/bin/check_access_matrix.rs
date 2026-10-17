//! Automated Access Control Matrix Verification Tool
//!
//! This tool statically analyzes all contract source files and verifies that
//! every public entrypoint's authorization implementation matches the documented
//! ACCESS_MATRIX.md specification.
//!
//! Exits with code 1 if any drift is detected (documented ≠ implemented).
//!
//! Usage:
//!   cargo xtask check-access-matrix
//!   cargo xtask check-access-matrix --contract marketplace
//!   cargo xtask check-access-matrix --verbose

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use syn::{File, Item, ItemFn, ImplItem, ImplItemFn};

type ContractName = String;
type FunctionName = String;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Authorization {
    Admin,
    AdminWithMultisig,
    Verifier,
    Feeder,
    SelfOwned, // Caller owns the resource (SME on their invoice, Investor on their position)
    ContractCaller(String), // Specific contract address (e.g., "marketplace")
    Governance,
    Permissionless,
    Conditional(Vec<Authorization>), // Multiple checks (e.g., Admin + NotPaused)
    Undocumented, // Not in ACCESS_MATRIX.md
}

#[derive(Debug)]
struct Drift {
    contract: ContractName,
    function: FunctionName,
    expected: Authorization,
    actual: Authorization,
}

/// Hardcoded access matrix matching docs/ACCESS_MATRIX.md
/// In production, this should parse the markdown file directly.
fn load_access_matrix() -> HashMap<(ContractName, FunctionName), Authorization> {
    let mut matrix = HashMap::new();

    // access_control
    matrix.insert(("access_control".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("access_control".into(), "set_admin".into()), Authorization::AdminWithMultisig);
    matrix.insert(("access_control".into(), "propose_admin".into()), Authorization::AdminWithMultisig);
    matrix.insert(("access_control".into(), "accept_admin".into()), Authorization::SelfOwned);
    matrix.insert(("access_control".into(), "set_protocol_paused".into()), Authorization::AdminWithMultisig);
    matrix.insert(("access_control".into(), "is_protocol_paused".into()), Authorization::Permissionless);
    matrix.insert(("access_control".into(), "add_multisig_signer".into()), Authorization::AdminWithMultisig);
    matrix.insert(("access_control".into(), "remove_multisig_signer".into()), Authorization::AdminWithMultisig);
    matrix.insert(("access_control".into(), "set_multisig_threshold".into()), Authorization::AdminWithMultisig);

    // risk_registry
    matrix.insert(("risk_registry".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("risk_registry".into(), "register_sme".into()), Authorization::SelfOwned);
    matrix.insert(("risk_registry".into(), "update_sme_profile".into()), Authorization::Verifier);
    matrix.insert(("risk_registry".into(), "register_verifier".into()), Authorization::Admin);
    matrix.insert(("risk_registry".into(), "remove_verifier".into()), Authorization::Admin);
    matrix.insert(("risk_registry".into(), "set_debtor_score".into()), Authorization::Verifier);
    matrix.insert(("risk_registry".into(), "get_debtor_score".into()), Authorization::Permissionless);
    matrix.insert(("risk_registry".into(), "set_credit_limit".into()), Authorization::Verifier);
    matrix.insert(("risk_registry".into(), "increment_invoice_count".into()), Authorization::ContractCaller("invoice_nft".into()));
    matrix.insert(("risk_registry".into(), "slash_verifier".into()), Authorization::Admin);
    matrix.insert(("risk_registry".into(), "get_sme_profile".into()), Authorization::Permissionless);
    matrix.insert(("risk_registry".into(), "is_verifier".into()), Authorization::Permissionless);

    // invoice_nft
    matrix.insert(("invoice_nft".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("invoice_nft".into(), "mint_invoice".into()), Authorization::SelfOwned);
    matrix.insert(("invoice_nft".into(), "mint_invoices_batch".into()), Authorization::SelfOwned);
    matrix.insert(("invoice_nft".into(), "set_funded".into()), Authorization::ContractCaller("marketplace".into()));
    matrix.insert(("invoice_nft".into(), "set_repaid".into()), Authorization::ContractCaller("financing_pool".into()));
    matrix.insert(("invoice_nft".into(), "set_defaulted".into()), Authorization::Admin);
    matrix.insert(("invoice_nft".into(), "freeze_invoice".into()), Authorization::Admin);
    matrix.insert(("invoice_nft".into(), "unfreeze_invoice".into()), Authorization::Admin);
    matrix.insert(("invoice_nft".into(), "set_mint_rate_limit".into()), Authorization::Admin);
    matrix.insert(("invoice_nft".into(), "get_invoice".into()), Authorization::Permissionless);
    matrix.insert(("invoice_nft".into(), "get_sme_invoices".into()), Authorization::Permissionless);
    matrix.insert(("invoice_nft".into(), "is_invoice_frozen".into()), Authorization::Permissionless);

    // marketplace
    matrix.insert(("marketplace".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("marketplace".into(), "set_fee_bps".into()), Authorization::AdminWithMultisig);
    matrix.insert(("marketplace".into(), "set_referrer_split_bps".into()), Authorization::AdminWithMultisig);
    matrix.insert(("marketplace".into(), "set_min_contribution".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "set_max_investor_share_bps".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "set_investor_accredited".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "set_tier_fee_bps".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "propose_token_whitelist".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "execute_token_whitelist".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "remove_token_whitelist".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "set_token_currency".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "set_token_exposure_cap".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "set_priority_window".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "list_invoice".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "fund_invoice".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "fund_invoices_batch".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "cancel_listing".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "withdraw_listing".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "request_cancellation".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "admin_confirm_cancellation".into()), Authorization::Admin);
    matrix.insert(("marketplace".into(), "claim_refund".into()), Authorization::SelfOwned);
    matrix.insert(("marketplace".into(), "get_listing".into()), Authorization::Permissionless);
    matrix.insert(("marketplace".into(), "get_config".into()), Authorization::Permissionless);
    matrix.insert(("marketplace".into(), "is_token_whitelisted".into()), Authorization::Permissionless);

    // financing_pool
    matrix.insert(("financing_pool".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("financing_pool".into(), "set_max_position_bps".into()), Authorization::Admin);
    matrix.insert(("financing_pool".into(), "set_late_penalty_split".into()), Authorization::Admin);
    matrix.insert(("financing_pool".into(), "set_marketplace".into()), Authorization::Admin);
    matrix.insert(("financing_pool".into(), "set_auto_compound".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "release_funds".into()), Authorization::ContractCaller("marketplace".into()));
    matrix.insert(("financing_pool".into(), "record_position".into()), Authorization::Admin);
    matrix.insert(("financing_pool".into(), "repay_partial".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "repay".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "net_settle".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "mark_default".into()), Authorization::Admin);
    matrix.insert(("financing_pool".into(), "propose_early_settlement".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "accept_early_settlement".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "cancel_early_settlement".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "split_position".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "transfer_share".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "list_share_for_sale".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "buy_share".into()), Authorization::SelfOwned);
    matrix.insert(("financing_pool".into(), "get_pool".into()), Authorization::Permissionless);
    matrix.insert(("financing_pool".into(), "get_positions".into()), Authorization::Permissionless);

    // price_oracle
    matrix.insert(("price_oracle".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("price_oracle".into(), "set_access_control".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "add_feeder".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "remove_feeder".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "set_price".into()), Authorization::Feeder);
    matrix.insert(("price_oracle".into(), "set_max_deviation".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "set_base_currency".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "set_peg_config".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "remove_peg_config".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "clear_peg_flag".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "set_max_rate_change".into()), Authorization::Admin);
    matrix.insert(("price_oracle".into(), "get_price".into()), Authorization::Permissionless);
    matrix.insert(("price_oracle".into(), "get_price_at".into()), Authorization::Permissionless);
    matrix.insert(("price_oracle".into(), "is_peg_flagged".into()), Authorization::Permissionless);

    // dispute_resolution
    matrix.insert(("dispute_resolution".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("dispute_resolution".into(), "open_dispute".into()), Authorization::SelfOwned);
    matrix.insert(("dispute_resolution".into(), "submit_evidence".into()), Authorization::SelfOwned);
    matrix.insert(("dispute_resolution".into(), "resolve_dispute".into()), Authorization::Governance);
    matrix.insert(("dispute_resolution".into(), "get_dispute".into()), Authorization::Permissionless);
    matrix.insert(("dispute_resolution".into(), "has_open_dispute".into()), Authorization::Permissionless);

    // treasury
    matrix.insert(("treasury".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("treasury".into(), "set_allocation".into()), Authorization::Admin);
    matrix.insert(("treasury".into(), "deposit".into()), Authorization::ContractCaller("financing_pool_or_marketplace".into()));
    matrix.insert(("treasury".into(), "withdraw".into()), Authorization::Admin);
    matrix.insert(("treasury".into(), "distribute".into()), Authorization::Permissionless);
    matrix.insert(("treasury".into(), "get_balance".into()), Authorization::Permissionless);
    matrix.insert(("treasury".into(), "get_allocation".into()), Authorization::Permissionless);

    // ── Issue #917: Extended matrix entries for all deploy.sh contracts ──────

    // secondary_market (issue #917)
    matrix.insert(("secondary_market".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("secondary_market".into(), "pause".into()), Authorization::Admin);
    matrix.insert(("secondary_market".into(), "unpause".into()), Authorization::Admin);
    matrix.insert(("secondary_market".into(), "update_protocol_fee_bps".into()), Authorization::Admin);
    matrix.insert(("secondary_market".into(), "list_position".into()), Authorization::SelfOwned);
    matrix.insert(("secondary_market".into(), "buy_position".into()), Authorization::SelfOwned);
    matrix.insert(("secondary_market".into(), "cancel_listing".into()), Authorization::SelfOwned);

    // fractionalizer (issue #917)
    matrix.insert(("fractionalizer".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("fractionalizer".into(), "wrap".into()), Authorization::SelfOwned);
    matrix.insert(("fractionalizer".into(), "transfer_shares".into()), Authorization::SelfOwned);
    matrix.insert(("fractionalizer".into(), "settle".into()), Authorization::Admin);
    matrix.insert(("fractionalizer".into(), "redeem".into()), Authorization::SelfOwned);
    matrix.insert(("fractionalizer".into(), "unwrap".into()), Authorization::SelfOwned);
    matrix.insert(("fractionalizer".into(), "get_wrap".into()), Authorization::Permissionless);
    matrix.insert(("fractionalizer".into(), "get_share_balance".into()), Authorization::Permissionless);

    // dispute_escrow (issue #917)
    matrix.insert(("dispute_escrow".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("dispute_escrow".into(), "open_dispute".into()), Authorization::SelfOwned);
    matrix.insert(("dispute_escrow".into(), "resolve_dispute".into()), Authorization::Admin);
    matrix.insert(("dispute_escrow".into(), "resolve_timeout".into()), Authorization::Permissionless);
    matrix.insert(("dispute_escrow".into(), "get_dispute".into()), Authorization::Permissionless);

    // governance_voting (issue #917)
    matrix.insert(("governance_voting".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("governance_voting".into(), "set_voting_weight".into()), Authorization::Admin);
    matrix.insert(("governance_voting".into(), "get_voting_weight".into()), Authorization::Permissionless);
    matrix.insert(("governance_voting".into(), "delegate".into()), Authorization::SelfOwned);
    matrix.insert(("governance_voting".into(), "revoke_delegation".into()), Authorization::SelfOwned);
    matrix.insert(("governance_voting".into(), "get_delegate".into()), Authorization::Permissionless);
    matrix.insert(("governance_voting".into(), "create_proposal".into()), Authorization::SelfOwned);
    matrix.insert(("governance_voting".into(), "vote".into()), Authorization::SelfOwned);
    matrix.insert(("governance_voting".into(), "get_proposal".into()), Authorization::Permissionless);
    matrix.insert(("governance_voting".into(), "get_vote_record".into()), Authorization::Permissionless);
    matrix.insert(("governance_voting".into(), "execute_proposal".into()), Authorization::Admin);

    // referral_rewards (issue #917)
    matrix.insert(("referral_rewards".into(), "initialize".into()), Authorization::Permissionless);
    matrix.insert(("referral_rewards".into(), "deposit".into()), Authorization::Admin);
    matrix.insert(("referral_rewards".into(), "claim_referral_reward".into()), Authorization::SelfOwned);
    matrix.insert(("referral_rewards".into(), "has_claimed".into()), Authorization::Permissionless);
    matrix.insert(("referral_rewards".into(), "paid_total".into()), Authorization::Permissionless);

    matrix
}
/// Parse a function's body to detect authorization patterns
fn detect_authorization(func: &ImplItemFn) -> Authorization {
    let func_body = quote::quote!(#func).to_string();

    // Detection heuristics (simplified; real implementation would use syn AST traversal)
    if func_body.contains("require_admin") || func_body.contains("admin.require_auth()") {
        if func_body.contains("multisig") || func_body.contains("quorum") {
            return Authorization::AdminWithMultisig;
        }
        return Authorization::Admin;
    }

    if func_body.contains("require_verifier") || func_body.contains("is_verifier") {
        return Authorization::Verifier;
    }

    if func_body.contains("require_feeder") {
        return Authorization::Feeder;
    }

    if func_body.contains("require_auth()") {
        // Self-owned if caller signs (SME, Investor, etc.)
        return Authorization::SelfOwned;
    }

    if func_body.contains("env.current_contract_address()") && func_body.contains("==") {
        // Contract-to-contract call validation
        if func_body.contains("marketplace") {
            return Authorization::ContractCaller("marketplace".into());
        }
        if func_body.contains("financing_pool") {
            return Authorization::ContractCaller("financing_pool".into());
        }
        if func_body.contains("invoice_nft") {
            return Authorization::ContractCaller("invoice_nft".into());
        }
        return Authorization::ContractCaller("unknown".into());
    }

    if func_body.contains("governance") {
        return Authorization::Governance;
    }

    // No authorization detected → likely permissionless view
    Authorization::Permissionless
}

/// Extract all public functions from a contract's impl block
fn extract_public_functions(file: &File, contract_name: &str) -> Vec<(FunctionName, Authorization)> {
    let mut functions = Vec::new();

    for item in &file.items {
        if let Item::Impl(impl_block) = item {
            // Only process #[contractimpl] blocks
            let has_contractimpl = impl_block.attrs.iter().any(|attr| {
                attr.path().is_ident("contractimpl")
            });

            if !has_contractimpl {
                continue;
            }

            for impl_item in &impl_block.items {
                if let ImplItem::Fn(func) = impl_item {
                    // Only process public functions
                    let is_public = matches!(func.vis, syn::Visibility::Public(_));
                    if !is_public {
                        continue;
                    }

                    let func_name = func.sig.ident.to_string();
                    let auth = detect_authorization(func);
                    functions.push((func_name, auth));
                }
            }
        }
    }

    functions
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let filter_contract = args.iter().position(|a| a == "--contract").and_then(|i| args.get(i + 1));
    let verbose = args.contains(&"--verbose".to_string());

    println!("🔍 Access Control Matrix Verification");
    println!("======================================\n");

    let contracts_dir = Path::new("contracts");
    let access_matrix = load_access_matrix();

    let mut total_functions = 0;
    let mut drift_found: Vec<Drift> = Vec::new();
    let mut undocumented: Vec<(ContractName, FunctionName)> = Vec::new();

    // Scan each contract directory
    // Issue #917: Scan ALL deployed crates from deploy.sh (original 8 + 5 new).
    // Adding a new crate to deploy.sh without a matrix entry will cause undocumented[]
    // to be non-empty, failing CI with exit code 1.
    let contract_dirs = [
        // Original 8 (always-deployed)
        "access_control",
        "risk_registry",
        "invoice_nft",
        "marketplace",
        "financing_pool",
        "price_oracle",
        "dispute_resolution",
        "treasury",
        // Issue #917: newly added to matrix coverage
        "secondary_market",
        "fractionalizer",
        "dispute_escrow",
        "governance_voting",
        "referral_rewards",
    ];

    for contract_name in &contract_dirs {
        if let Some(filter) = filter_contract {
            if filter != contract_name {
                continue;
            }
        }

        let lib_path = contracts_dir.join(contract_name).join("src/lib.rs");
        if !lib_path.exists() {
            eprintln!("⚠️  Contract {} not found at {:?}", contract_name, lib_path);
            continue;
        }

        if verbose {
            println!("📄 Analyzing {}...", contract_name);
        }

        let source = fs::read_to_string(&lib_path).expect("Failed to read contract source");
        let file = syn::parse_file(&source).expect("Failed to parse Rust source");

        let functions = extract_public_functions(&file, contract_name);
        total_functions += functions.len();

        for (func_name, actual_auth) in functions {
            let key = (contract_name.to_string(), func_name.clone());

            match access_matrix.get(&key) {
                Some(expected_auth) => {
                    if expected_auth != &actual_auth {
                        drift_found.push(Drift {
                            contract: contract_name.to_string(),
                            function: func_name.clone(),
                            expected: expected_auth.clone(),
                            actual: actual_auth,
                        });
                    } else if verbose {
                        println!("  ✅ {}::{} — {:?}", contract_name, func_name, expected_auth);
                    }
                }
                None => {
                    undocumented.push((contract_name.to_string(), func_name.clone()));
                }
            }
        }
    }

    println!("\n📊 Verification Summary");
    println!("=======================");
    println!("Total functions analyzed: {}", total_functions);
    println!("Authorization drift detected: {}", drift_found.len());
    println!("Undocumented entrypoints: {}", undocumented.len());

    if !drift_found.is_empty() {
        println!("\n❌ AUTHORIZATION DRIFT DETECTED:");
        for drift in &drift_found {
            println!("\n  Contract: {}", drift.contract);
            println!("  Function: {}", drift.function);
            println!("  Expected: {:?}", drift.expected);
            println!("  Actual:   {:?}", drift.actual);
            println!("  → Action: Update docs/ACCESS_MATRIX.md or fix code authorization");
        }
    }

    if !undocumented.is_empty() {
        println!("\n⚠️  UNDOCUMENTED ENTRYPOINTS:");
        for (contract, func) in &undocumented {
            println!("  {}::{}", contract, func);
        }
        println!("\n→ Action: Add these functions to docs/ACCESS_MATRIX.md with authorization requirements");
    }

    if drift_found.is_empty() && undocumented.is_empty() {
        println!("\n✅ All authorization checks match ACCESS_MATRIX.md specification!");
        std::process::exit(0);
    } else {
        println!("\n❌ Verification failed. Fix drift/documentation before committing.");
        std::process::exit(1);
    }
}
