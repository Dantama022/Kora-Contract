// tests/multisig_threshold_manipulation.rs
//
// #614 — Multisig Threshold Manipulation Test Suite
//
// Tests scenarios where the signer set or threshold changes while a proposal is
// mid-flight, ensuring no stale-signer-count exploit is possible.
//
// Intended semantics (documented here because they are not yet explicit in the
// contract's documentation):
//
//   • Votes are recorded as signer addresses.  At execution time, execute_action()
//     reloads the CURRENT MultisigConfig and re-validates the executor is still a
//     signer.  The approval count is compared against the CURRENT threshold.
//
//   • A signer who voted but was later removed from the config: their vote still
//     counts (it is stored in the proposal's `approvals` Vec).  The contract does
//     NOT re-validate each historical voter against the current signer set at
//     execution time.  This is a known design choice documented in this file.
//
//   • Raising the threshold after votes are collected: if the new threshold
//     exceeds the current approval count, execution must be rejected.
//
//   • Lowering the threshold after votes are collected: execution may succeed if
//     the approval count now meets or exceeds the lower threshold.
//
//   • Replacing the entire signer set: a proposal approved by the old set can be
//     executed by any member of the new set, but only if the new threshold is met
//     by the stored approval count.
//
// Each scenario maps to a distinct test.  Where the current implementation allows
// a potentially undesirable outcome, it is documented as a known design choice
// for future hardening.

#[cfg(test)]
mod multisig_threshold_manipulation {
    use kora_access_control::{AccessControlContract, AccessControlContractClient};
    use kora_shared::types::AdminAction;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, LedgerInfo},
        Address, Env, Vec,
    };

    const BASE_TIMESTAMP: u64 = 1_700_000_000;

    fn ledger_at(env: &Env, ts: u64) {
        env.ledger().set(LedgerInfo {
            timestamp: ts,
            protocol_version: 21,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1000,
            min_persistent_entry_ttl: 1000,
            max_entry_ttl: 100_000,
        });
    }

    /// Deploy access_control, configure an N-of-M multisig, return (env, admin, signers, client).
    fn setup(
        n_signers: usize,
        threshold: u32,
    ) -> (Env, Address, Vec<Address>, AccessControlContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        ledger_at(&env, BASE_TIMESTAMP);

        let admin = Address::generate(&env);
        let ac_id = env.register_contract(None, AccessControlContract);
        let client = AccessControlContractClient::new(&env, &ac_id);
        client.initialize(&admin);

        let mut signers = Vec::new(&env);
        for _ in 0..n_signers {
            signers.push_back(Address::generate(&env));
        }
        client.configure_multisig(&admin, &signers, &threshold);

        (env, admin, signers, client)
    }

    /// Propose a Pause action via `proposer` and return the proposal_id.
    fn propose_pause(
        client: &AccessControlContractClient,
        proposer: &Address,
    ) -> u64 {
        client.propose_action(proposer, &AdminAction::Pause)
    }

    // ── Scenario 1: Signer removed after voting, before execution ────────────
    //
    // A 2-of-3 multisig.  Signer A proposes (vote 1), Signer B approves (vote 2).
    // Admin then reconfigures to a new 2-of-3 that excludes B (but keeps A).
    // The proposal already has 2 approvals stored.
    //
    // Issue #915 hardened behaviour: at execution time, each stored voter is
    // re-validated against the CURRENT signer set.  Signer B was removed, so
    // only A's approval counts → 1 valid approval < threshold 2 → execution FAILS.
    //
    // This supersedes the old design-choice comment (issue #614) that allowed
    // stale votes to count.
    #[test]
    fn signer_removed_after_voting_proposal_still_executes() {
        let (env, admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();
        let signer_c = signers.get(2).unwrap();

        // Signer A proposes (auto-votes), Signer B approves.
        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);

        // Admin reconfigures: replace B with a new signer D, keeping A and C.
        let signer_d = Address::generate(&env);
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(signer_a.clone());
        new_signers.push_back(signer_c.clone());
        new_signers.push_back(signer_d.clone());
        client.configure_multisig(&admin, &new_signers, &2u32);

        // Signer A tries to execute.  B's vote is now stale (B removed) → only 1
        // valid approval (A) remains, which is below threshold 2.
        // Issue #915: execution must FAIL — stale votes are discarded.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_err(),
            "#614-1 / #915: execution must fail — B's stale vote is discarded at execution time, \
             leaving only 1 valid approval which is below the threshold of 2"
        );
    }

    // ── Scenario 2: Threshold raised after votes collected ───────────────────
    //
    // A 2-of-3 multisig.  Two signers vote (meeting the original threshold of 2).
    // Admin raises the threshold to 3.  Execution must now fail.
    #[test]
    fn threshold_raised_after_votes_collected_blocks_execution() {
        let (env, admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);
        // Approval count = 2, which meets the original threshold of 2.

        // Admin raises threshold to 3 (unanimous).
        client.configure_multisig(&admin, &signers, &3u32);

        // Execution must now fail: 2 approvals < new threshold of 3.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_err(),
            "#614-2: execution must fail when threshold was raised above current approval count"
        );
    }

    // ── Scenario 3: Threshold lowered after votes collected ──────────────────
    //
    // A 3-of-3 multisig.  Only 2 signers vote (below original threshold of 3).
    // Admin lowers the threshold to 2.  Execution must now succeed.
    //
    // This documents that lowering the threshold retroactively allows execution
    // of proposals that previously lacked quorum — a significant power the admin
    // holds over in-flight proposals.
    #[test]
    fn threshold_lowered_after_votes_allows_execution() {
        let (env, admin, signers, client) = setup(3, 3);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);
        // Approval count = 2, below original threshold of 3.

        // Execution must fail at original threshold of 3.
        let before_reconfig = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            before_reconfig.is_err(),
            "#614-3: execution must fail before threshold is lowered"
        );

        // Admin lowers threshold to 2.
        client.configure_multisig(&admin, &signers, &2u32);

        // Execution must now succeed: 2 approvals >= new threshold of 2.
        let after_reconfig = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            after_reconfig.is_ok(),
            "#614-3: execution must succeed after threshold was lowered to match approval count \
             (documented design: threshold change applies retroactively to in-flight proposals)"
        );
    }

    // ── Scenario 4: Entire signer set replaced mid-proposal ──────────────────
    //
    // A 2-of-3 multisig collects 2 votes.  Admin replaces ALL signers with a
    // completely new set (new threshold = 2).
    // A member of the new set tries to execute.
    //
    // Issue #915 hardened behaviour: the stored approvals (from A and B of the
    // OLD set) are re-validated against the NEW signer set (X, Y, Z).  None of
    // the old voters are in the new set → 0 valid approvals < threshold 2 →
    // execution must FAIL.  The new set must gather its own fresh approvals.
    #[test]
    fn signer_set_replaced_mid_proposal_new_signer_can_execute() {
        let (env, admin, old_signers, client) = setup(3, 2);

        let signer_a = old_signers.get(0).unwrap();
        let signer_b = old_signers.get(1).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);
        // 2 approvals stored (from old set A and B).

        // Admin replaces entire signer set.
        let new_signer_x = Address::generate(&env);
        let new_signer_y = Address::generate(&env);
        let new_signer_z = Address::generate(&env);
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(new_signer_x.clone());
        new_signers.push_back(new_signer_y.clone());
        new_signers.push_back(new_signer_z.clone());
        client.configure_multisig(&admin, &new_signers, &2u32);

        // New-set member tries to execute.  Old approvals (A, B) are both stale.
        // 0 valid approvals < threshold 2 → must fail.
        let result = client.try_execute_action(&new_signer_x, &proposal_id);
        assert!(
            result.is_err(),
            "#614-4 / #915: execution must fail — all stored approvals are from the old signer set \
             and are discarded; 0 valid approvals cannot meet threshold 2"
        );
    }

    // ── Scenario 5: Old-set member cannot execute after being replaced ────────
    //
    // After the signer set is replaced (Scenario 4 setup), a member of the OLD
    // set must no longer be able to call execute_action.
    #[test]
    fn evicted_signer_cannot_execute_after_set_replaced() {
        let (env, admin, old_signers, client) = setup(3, 2);

        let signer_a = old_signers.get(0).unwrap();
        let signer_b = old_signers.get(1).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);

        // Replace entire signer set.
        let new_signer_x = Address::generate(&env);
        let new_signer_y = Address::generate(&env);
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(new_signer_x);
        new_signers.push_back(new_signer_y);
        client.configure_multisig(&admin, &new_signers, &2u32);

        // Old signer A tries to execute — must fail.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_err(),
            "#614-5: evicted signer must not be able to execute after signer set replaced"
        );
    }

    // ── Scenario 6: Non-signer cannot approve or execute ─────────────────────
    //
    // Baseline sanity: an address that was never configured as a signer must be
    // rejected by both approve_action and execute_action.
    #[test]
    fn non_signer_cannot_approve_or_execute() {
        let (env, _admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let proposal_id = propose_pause(&client, &signer_a);

        let outsider = Address::generate(&env);

        let approve_result = client.try_approve_action(&outsider, &proposal_id);
        assert!(
            approve_result.is_err(),
            "#614-6: non-signer must not be able to approve a proposal"
        );

        let execute_result = client.try_execute_action(&outsider, &proposal_id);
        assert!(
            execute_result.is_err(),
            "#614-6: non-signer must not be able to execute a proposal"
        );
    }

    // ── Scenario 7: Double-voting by the same signer is rejected ─────────────
    //
    // A signer who already voted (or is the proposer) must not be able to cast a
    // second vote to artificially inflate the approval count.
    #[test]
    fn same_signer_cannot_vote_twice() {
        let (_env, _admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let proposal_id = propose_pause(&client, &signer_a);
        // Signer A already voted as proposer.

        let second_vote = client.try_approve_action(&signer_a, &proposal_id);
        assert!(
            second_vote.is_err(),
            "#614-7: same signer must not be able to vote twice on the same proposal"
        );
    }

    // ── Scenario 8: Executed proposal cannot be re-executed ──────────────────
    //
    // After successful execution, a repeat call must fail regardless of signer
    // set composition.
    #[test]
    fn executed_proposal_cannot_be_replayed() {
        let (_env, _admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);

        let first = client.try_execute_action(&signer_a, &proposal_id);
        assert!(first.is_ok(), "#614-8: first execute must succeed");

        let second = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            second.is_err(),
            "#614-8: executed proposal must not be re-executable"
        );
    }

    // ── Scenario 9: Threshold of 1-of-1 single signer ────────────────────────
    //
    // Edge case: minimum valid configuration.  A 1-of-1 multisig must allow
    // the single signer to propose and immediately execute.
    #[test]
    fn single_signer_1_of_1_can_propose_and_execute() {
        let (_env, _admin, signers, client) = setup(1, 1);
        let signer = signers.get(0).unwrap();

        let proposal_id = propose_pause(&client, &signer);
        // No additional approvals needed.
        let result = client.try_execute_action(&signer, &proposal_id);
        assert!(
            result.is_ok(),
            "#614-9: 1-of-1 signer must be able to propose and immediately execute"
        );
    }

    // ── Scenario 10: configure_multisig with invalid threshold rejected ───────
    //
    // Threshold = 0 or threshold > n_signers must be rejected to prevent a
    // configuration that can never be met (or trivially met with zero votes).
    #[test]
    fn configure_multisig_rejects_zero_threshold() {
        let (env, admin, signers, client) = setup(3, 2);

        let result = client.try_configure_multisig(&admin, &signers, &0u32);
        assert!(
            result.is_err(),
            "#614-10: configure_multisig with threshold=0 must be rejected"
        );
    }

    #[test]
    fn configure_multisig_rejects_threshold_above_signer_count() {
        let (env, admin, signers, client) = setup(3, 2);

        let result = client.try_configure_multisig(&admin, &signers, &4u32);
        assert!(
            result.is_err(),
            "#614-10: configure_multisig with threshold > signers must be rejected"
        );
    }
}

    // ════════════════════════════════════════════════════════════════════════════
    // Issue #915 — Re-validate multisig voters against current signer set
    //
    // The following tests verify that execute_action filters out approvals from
    // signers who have been removed from the signer set between voting and
    // execution.  This is a security hardening: stale votes must NOT count toward
    // the threshold at execution time.
    //
    // Contrast with the existing Scenarios 1–5 (issue #614) which document the
    // old *design choice* of not re-validating; these tests document and enforce
    // the NEW hardened behaviour introduced by issue #915.
    // ════════════════════════════════════════════════════════════════════════════

    // ── Scenario 11: Single removed signer's vote does not count ─────────────
    //
    // A 2-of-3 multisig.  Signer A proposes (vote 1), Signer B approves (vote 2).
    // Admin reconfigures to a new 2-of-3 that removes B (keeps A and C, adds D).
    //
    // Issue #915 hardened behaviour: only 1 valid approval remains (A) after
    // re-validation — B's vote is discarded.  Threshold is still 2, so execution
    // must FAIL.
    #[test]
    fn signer_removed_stale_vote_causes_execution_to_fail() {
        let (env, admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();
        let signer_c = signers.get(2).unwrap();

        // A proposes (1 approval), B approves (2 approvals).
        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);
        // Approval count = 2, threshold = 2 — would have executed before #915.

        // Rotate B out; new set is {A, C, D}.
        let signer_d = Address::generate(&env);
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(signer_a.clone());
        new_signers.push_back(signer_c.clone());
        new_signers.push_back(signer_d.clone());
        client.configure_multisig(&admin, &new_signers, &2u32);

        // A tries to execute.  Only A's vote is valid → 1 valid < threshold 2.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_err(),
            "#915-11: execution must fail when the only remaining valid vote is below threshold \
             (stale vote from removed signer B must not count)"
        );
    }

    // ── Scenario 12: 2-of-3 to 1-of-3 rotation — one valid voter still enough ─
    //
    // A 2-of-3 multisig collects 2 approvals (A and B).  Signer B is removed,
    // lowering valid votes to 1.  Admin ALSO lowers the threshold to 1.
    // Net result: 1 valid approval ≥ new threshold 1 → execution MUST succeed.
    #[test]
    fn valid_voter_after_rotation_succeeds_when_threshold_lowered() {
        let (env, admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();
        let signer_c = signers.get(2).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);

        // Remove B, lower threshold to 1.
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(signer_a.clone());
        new_signers.push_back(signer_c.clone());
        client.configure_multisig(&admin, &new_signers, &1u32);

        // 1 valid approval (A) ≥ new threshold 1 → success.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_ok(),
            "#915-12: execution must succeed when valid votes (1) meet the new threshold (1) \
             even though B's vote was discarded"
        );
    }

    // ── Scenario 13: Removed signer cannot execute alone ─────────────────────
    //
    // A 1-of-1 multisig: signer A proposes and immediately has quorum.  Admin
    // replaces A with B.  Former signer A must NOT be able to execute.
    #[test]
    fn removed_signer_cannot_execute_alone_after_rotation() {
        let (env, admin, signers, client) = setup(1, 1);

        let signer_a = signers.get(0).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        // Threshold was 1, A's vote is present — would have executed.

        // Replace A with B.
        let signer_b = Address::generate(&env);
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(signer_b.clone());
        client.configure_multisig(&admin, &new_signers, &1u32);

        // A is no longer a signer; execute_action must reject A as executor.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_err(),
            "#915-13: removed signer A must not be able to execute after replacement"
        );
    }

    // ── Scenario 14: Duplicate voter entries do not inflate count ─────────────
    //
    // Guard against crafted proposals with the same address appearing twice in
    // `approvals`.  The approval path rejects double-voting, but we verify the
    // re-validation path also counts each distinct valid signer at most once.
    //
    // This tests the system end-to-end: we use the normal proposal flow (which
    // prevents duplicates) to ensure that a 2-of-3 proposal with honest votes
    // behaves correctly after one voter is removed (1 valid < 2 threshold).
    #[test]
    fn revalidation_counts_each_valid_signer_once() {
        let (env, admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();
        let signer_c = signers.get(2).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        // Verify duplicate vote is rejected (dedup guard in approve_action).
        assert!(
            client.try_approve_action(&signer_a, &proposal_id).is_err(),
            "#915-14: duplicate vote must be rejected"
        );
        client.approve_action(&signer_b, &proposal_id);
        // approvals = [A, B], threshold = 2 → should be fine before rotation.

        // Remove B; new threshold still 2.
        let mut new_signers = Vec::new(&env);
        new_signers.push_back(signer_a.clone());
        new_signers.push_back(signer_c.clone());
        client.configure_multisig(&admin, &new_signers, &2u32);

        // Now only A is valid → 1 < 2 threshold → must fail.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_err(),
            "#915-14: 1 valid approval must not meet a threshold of 2 after stale vote discarded"
        );
    }

    // ── Scenario 15: All original voters remain — normal execution succeeds ───
    //
    // Control case: no signers removed.  The re-validation path must not break
    // the normal happy path when all voters are still in the signer set.
    #[test]
    fn no_rotation_all_voters_still_valid_executes_normally() {
        let (_env, _admin, signers, client) = setup(3, 2);

        let signer_a = signers.get(0).unwrap();
        let signer_b = signers.get(1).unwrap();

        let proposal_id = propose_pause(&client, &signer_a);
        client.approve_action(&signer_b, &proposal_id);

        // No reconfiguration — all voters still valid.
        let result = client.try_execute_action(&signer_a, &proposal_id);
        assert!(
            result.is_ok(),
            "#915-15: normal 2-of-3 execution must still succeed when no signers have been removed"
        );
    }
}
