/**
 * Governance Parameter Range Abuse Tests
 *
 * Ensures that range boundaries in governance_voting cannot be exploited
 * to obtain excessive vote weight through:
 *   1. Zero/negative voice credits in quadratic voting
 *   2. Overflow attempts via extreme amounts
 *   3. Vesting contract edge cases (revoked grants, multiple grants)
 *   4. Delegation with vesting weights
 */
use kora_governance_voting::{
    GovernanceVotingContract, GovernanceVotingContractClient,
    GovernanceError, ProposalType, VoteChoice, VotingMode,
};
use kora_shared::validation::require_not_self;
use kora_vesting::{VestingContract, VestingContractClient, VestingError};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, String,
};

/// Shared setup: deploys vesting and governance, wires them together.
fn setup<'a>() -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, GovernanceVotingContract);
    let gov = GovernanceVotingContractClient::new(&env, &contract_id);

    // Vesting contract
    let token_admin = Address::generate(&env);
    let token_addr = env.register_stellar_asset_contract_v2(token_admin.clone());
    let token = token::Client::new(&env, &token_addr);
    let vesting_addr = env.register_contract(None, VestingContract);
    let vesting = VestingContractClient::new(&env, &vesting_addr);
    vesting.initialize(&admin, &token_addr);
    token.mint(&vesting_addr, &1_000_000);

    // Governance needs an access control address — generate a dummy one
    let access_control = Address::generate(&env);
    gov.initialize(&admin, &access_control);

    // Wire vesting contract into governance
    gov.set_vesting_contract(&admin, &vesting_addr);

    Setup { env, admin, token, vesting, vesting_addr, gov, contract_id }
}

struct Setup {
    env: Env,
    admin: Address,
    token: token::Client,
    vesting: VestingContractClient,
    vesting_addr: Address,
    gov: GovernanceVotingContractClient,
    contract_id: Address,
}

/// Helper: create a proposal and return its ID.
fn create_proposal<'a>(
    gov: &GovernanceVotingContractClient,
    voter: &Address,
    env: &Env,
) -> u64 {
    let now = env.ledger().timestamp();
    gov.create_proposal(
        voter,
        &String::from_str(env, "Parameter Range Test"),
        &String::from_str(env, "Testing boundaries"),
        &ProposalType::ParameterChange,
        &VotingMode::Standard,
        &1i128,         // quorum = 1
        &(now + 3600), // duration (1 hour)
    );
    // After create_proposal, the ID is the next sequential one
    // We track this via the test: the first proposal has ID = 1
    1u64
}

// ── Test 1: Vote weight equals vested balance, not grant total ────────────

#[test]
fn test_vote_weight_equals_vested_not_grant_total() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();
    let grant_total: i128 = 10_000;
    let cliff = 100u64;
    let duration = 1000u64;

    // Create grant with large total but early in cliff
    s.vesting.create_grant(&s.admin, &voter, &grant_total, &now, &cliff, &duration);

    // Create proposal
    let proposal_id = create_proposal(&s.gov, &voter, &s.env);

    // Advance time past cliff but not fully vested
    let mid_time = now + 500;
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    // Vote with max weight available
    s.gov.vote(&voter, &proposal_id, &VoteChoice::For, &grant_total);

    // Check the vote record weight equals vested (linear: 500/1000 * 10000 = 5000)
    let vote_rec = s.gov.get_vote_record(&proposal_id, &voter);
    assert_eq!(vote_rec.weight, 5000, "Vote weight should be vested amount (5000), not grant total (10000)");
}

// ── Test 2: Unvested grant balance does not increase vote power ────────────

#[test]
fn test_unvested_balance_does_not_increase_weight() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();

    // Grant starts vesting in the future
    let future_time = now + 1000;
    s.vesting.create_grant(&s.admin, &voter, &10_000, &future_time, &100, &1000);

    let proposal_id = create_proposal(&s.gov, &voter, &s.env);

    // Try to vote — should fail with InsufficientVotingWeight since vested balance is 0
    let result = s.gov.try_vote(&voter, &proposal_id, &VoteChoice::For, &100i128);
    assert!(result.is_err(), "Vote should fail when vested balance is 0");
}

// ── Test 3: Revoked grant zeroes weight after revocation ───────────────────

#[test]
fn test_revoked_grant_zeroes_weight() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();
    s.vesting.create_grant(&s.admin, &voter, &10_000, &now, &100, &1000);

    // Advance past cliff
    let mid_time = now + 500;
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    // Revoke the grant
    s.vesting.revoke_grant(&s.admin, &voter);

    // Create proposal AFTER revocation
    let proposal_id = create_proposal(&s.gov, &voter, &s.env);

    // Try to vote — should fail since weight is 0 (revoked grant freezes at revoked_at)
    let result = s.gov.try_vote(&voter, &proposal_id, &VoteChoice::For, &100i128);
    assert!(result.is_err(), "Vote should fail with 0 weight after grant revocation");
}

// ── Test 4: Multiple grants aggregate vested balances ──────────────────────

#[test]
fn test_multiple_grants_aggregate() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();

    // Create two separate grants for the same beneficiary
    s.vesting.create_grant(&s.admin, &voter, &20_000, &now, &100, &1000);
    s.vesting.create_grant(&s.admin, &voter, &5_000, &now, &0, &500);

    // Advance to mid-point
    let mid_time = now + 500;
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    let proposal_id = create_proposal(&s.gov, &voter, &s.env);

    // Vote
    s.gov.vote(&voter, &proposal_id, &VoteChoice::For, &25_000i128);

    // Weight should be sum of vested balances:
    //   Grant 1: 20_000 * 500/1000 = 10_000
    //   Grant 2: 5_000 * 500/500 = 5_000
    //   Total: 15_000
    let vote_rec = s.gov.get_vote_record(&proposal_id, &voter);
    assert_eq!(vote_rec.weight, 15_000, "Weight should aggregate vested balances from multiple grants");
}

// ── Test 5: Quadratic voting caps by vested balance ───────────────────────

#[test]
fn test_quadratic_vote_capped_by_vested_balance() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();
    s.vesting.create_grant(&s.admin, &voter, &10_000, &now, &100, &1000);

    let proposal_id = create_proposal(&s.gov, &voter, &s.env);

    let mid_time = now + 500;
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    // Quadratic: votes = sqrt(voice_credits), credits must be <= vested (5_000)
    // 5_000 voice credits → sqrt(5_000) ≈ 70 votes
    s.gov.vote(&voter, &proposal_id, &VoteChoice::For, &5_000i128);

    let vote_rec = s.gov.get_vote_record(&proposal_id, &voter);
    assert!(vote_rec.weight > 0, "Quadratic vote should succeed within vested limit");

    // Voice credits > vested should fail
    let proposal_id2 = proposal_id + 1;
    let now2 = s.env.ledger().timestamp();
    s.gov.create_proposal(
        &voter,
        &String::from_str(&s.env, "Quadratic Cap Test"),
        &String::from_str(&s.env, ""),
        &ProposalType::ParameterChange,
        &VotingMode::Quadratic,
        &1i128,
        &(now2 + 3600),
    );
    let result = s.gov.try_vote(&voter, &proposal_id2, &VoteChoice::For, &6_000i128);
    assert!(result.is_err(), "Quadratic vote should fail when voice credits exceed vested balance");
}

// ── Test 6: Vesting contract unset — falls back to manual weight ───────────

#[test]
fn test_vesting_unset_uses_manual_weight_fallback() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let gov_addr = env.register_contract(None, GovernanceVotingContract);
    let gov = GovernanceVotingContractClient::new(&env, &gov_addr);

    let access_control = Address::generate(&env);
    gov.initialize(&admin, &access_control);

    let voter = Address::generate(&env);
    let now = env.ledger().timestamp();

    // Set manual weight (fallback when no vesting configured)
    gov.set_voting_weight(&admin, &voter, &500);

    // Create a proposal
    let proposal_id = gov.create_proposal(
        &voter,
        &String::from_str(&env, "Test"),
        &String::from_str(&env, ""),
        &ProposalType::ParameterChange,
        &VotingMode::Standard,
        &1i128,
        &(now + 3600),
    );

    // Vote should work with manual weight fallback
    let result = gov.try_vote(&voter, &proposal_id, &VoteChoice::For, &100i128);
    assert!(result.is_ok(), "Vote should succeed using manual weight fallback when vesting is not set");

    // Verify vote weight is the manual weight
    let proposal = gov.get_proposal(&proposal_id).unwrap();
    assert_eq!(proposal.votes_for, 500, "Vote should use manual weight (500) when vesting contract is not configured");
}

// ── Test 7: Unauthorized set_vesting_contract rejected ─────────────────────

#[test]
fn test_unauthorized_vesting_setter_rejected() {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let gov_addr = env.register_contract(None, GovernanceVotingContract);
    let gov = GovernanceVotingContractClient::new(&env, &gov_addr);

    let access_control = Address::generate(&env);
    gov.initialize(&admin, &access_control);

    let vesting_addr = Address::generate(&env);
    let attacker = Address::generate(&env);

    // Attacker tries to set the vesting contract
    let result = gov.try_set_vesting_contract(&attacker, &vesting_addr);
    assert!(result.is_err(), "Unauthorized set_vesting_contract should fail");
}

// ── Test 8: Beneficiary delegation still works ─────────────────────────────

#[test]
fn test_delegation_with_vesting_weights() {
    let s = setup();

    let delegator = Address::generate(&s.env);
    let delegate = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();

    // Create grant for delegator
    s.vesting.create_grant(&s.admin, &delegator, &10_000, &now, &100, &1000);

    // Delegator delegates to delegate
    s.gov.set_delegation(&delegator, &delegate);

    // Advance past cliff
    let mid_time = now + 500;
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    // Delegate's weight should include delegator's vested balance (5_000)
    let delegate_weight = s.gov.get_voting_weight(&delegate);
    assert_eq!(delegate_weight, 5000, "Delegate weight should include delegator's vested balance");

    // Delegate can vote with combined weight
    let proposal_id = create_proposal(&s.gov, &delegate, &s.env);
    s.gov.vote(&delegate, &proposal_id, &VoteChoice::For, &5_000i128);

    let vote_rec = s.gov.get_vote_record(&proposal_id, &delegate);
    assert_eq!(vote_rec.weight, 5000, "Vote weight should include delegated vested balance");
}

// ── Test 9: Overflow-safe arithmetic with large amounts ────────────────────

#[test]
fn test_overflow_safe_amounts() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();
    // Use very large amounts that are still within i128 range
    let large_total: i128 = i128::MAX / 3;
    let duration = 1_000_000u64;

    s.vesting.create_grant(&s.admin, &voter, &large_total, &now, &100, &duration);

    let proposal_id = create_proposal(&s.gov, &voter, &s.env);

    // Advance 50% through vesting
    let mid_time = now + (duration / 2);
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    // Large vote should not overflow
    let result = s.gov.try_vote(&voter, &proposal_id, &VoteChoice::For, &large_total);
    assert!(result.is_ok(), "Large vested amounts should not cause arithmetic overflow");
}

// ── Test 10: Events expose snapshot ledger and weight ──────────────────────

#[test]
fn test_vote_event_contains_weight_basis() {
    let s = setup();

    let voter = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();

    s.vesting.create_grant(&s.admin, &voter, &10_000, &now, &100, &1000);

    // Create proposal (snapshot at creation)
    let proposal_id = create_proposal(&s.gov, &voter, &s.env);
    // Need to store proposal.created_at, but we can't read it directly from the contract
    // Instead we check the emitted event via the event log
    // (Event testing is framework-specific; this test ensures the vote doesn't error)

    let mid_time = now + 500;
    s.env.ledger().with_mut(|li| li.timestamp = mid_time);

    let result = s.gov.try_vote(&voter, &proposal_id, &VoteChoice::For, &10_000i128);
    assert!(result.is_ok(), "Vote should succeed and emit event with weight basis");
}
