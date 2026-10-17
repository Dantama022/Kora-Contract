# Kora Governance Voting Contract

A comprehensive on-chain governance system for the Kora Protocol, supporting vote delegation and quadratic voting for community-driven decision making.

## Deployment inputs and workspace CI

`GovernanceVotingContractClient` is the generated Rust client for this crate.
Initialize it with the admin address, the access-control contract address, and
optionally a vesting contract address (see [Weight Sources](#weight-sources)).
For the forum bridge, `GOVERNANCE_CONTRACT_ID` must be this deployment's address,
and `SOROBAN_RPC_URL` selects its network's RPC endpoint. See the bridge's
[configuration and client mapping](../../services/governance-sync/README.md#configuration).

CI runs `cargo test -p kora-governance-voting` as a dedicated matrix entry (#912),
with workspace fmt/clippy/tests also including the crate. Persistent proposal,
vote, weight, and delegation writes extend TTL using the contract's bump helper.
The cross-contract call to the vesting contract is gas-metered; cache snapshot
results when possible during a single vote transaction.

## Weight Sources

Vote weight is computed from the beneficiary's **vested token balance** at the
proposal's `created_at` snapshot timestamp, queried via a read-only cross-contract
call to the Vesting contract (`get_vested_balance_at`). The admin configures the
vesting contract address through `set_vesting_contract(admin, vesting_address)`.

**Weight formula:**

```
weight = vested_balance(voter, proposal.created_at)
```

- If no grant exists for the voter, weight is 0.
- If the grant was revoked before or at the snapshot time, weight is the
  vested amount up to the revocation timestamp.
- Multiple grants aggregate their vested balances.
- **Unvested grant balance does not increase vote power.**
- The old manual `set_voting_weight` storage is used only as a fallback when
  no vesting contract is configured (see below).

**Fallback behavior:** If `set_vesting_contract` has not been called, the
`vote()` entrypoint falls back to the manually set `VotingWeight` storage
(backward compatible). To enforce on-chain vesting weights, the admin must
configure the vesting contract address. Once configured, manual weights are
ignored and only vested balances are used.

**Delegation** aggregates the vested balances of all delegators. The delegate's
total voting weight is their own vested balance plus the sum of each delegator's
vested balance at the current ledger timestamp.

## Events

Every vote emits a `voteCast` event with the following topics and fields:

**Topics:**
- `voteCast` — event discriminator
- `SCHEMA_V` — schema version for indexers

**Fields (tuple):**
1. `proposal_id` (u64) — the proposal being voted on
2. `voter` (Address) — the voting address
3. `weight` (i128) — the effective vote weight applied
4. `voice_credits` (i128) — voice credits spent (0 for standard mode)
5. `choice` (u32) — 0=For, 1=Against, 2=Abstain
6. `snapshot_at` (u64) — the proposal's `created_at` timestamp (vesting snapshot)
7. `schema_v` (u32) — event schema version

Off-chain indexers should use `snapshot_at` together with the voter address to
reconcile vote weight against vesting contract state.

## Features

### 1. Vote Delegation

Stakeholders can delegate their voting weight to trusted representatives without transferring underlying stake ownership.

**Key Properties:**
- **Single-level delegation**: No transitive/chained delegation
- **Revocable at any time**: Delegators maintain full control
- **Ownership retained**: Underlying stake never transfers
- **Direct vote precedence**: Delegator's direct vote overrides delegation

**Prevents:**
- Circular delegation (A→B→A)
- Self-delegation
- Double-counting

**Use Cases:**
- Small stakeholders who lack time/expertise
- Community representatives accumulating voting power
- Flexible governance participation

### 2. Quadratic Voting

Optional voting mode for community sentiment proposals where vote cost scales quadratically.

**How it Works:**
- Voice credits derived from voting weight (stake)
- Votes = √(voice_credits_spent)
- Examples:
  - 1 credit → 1 vote
  - 4 credits → 2 votes
  - 9 credits → 3 votes
  - 25 credits → 5 votes
  - 100 credits → 10 votes

**Proposal Type Restrictions:**
- ✅ **Sentiment proposals**: Community preference, protocol direction
- ❌ **Financial proposals**: Must use standard stake-weighted voting

**Benefits:**
- Better captures broad community preference
- Reduces disproportionate influence of large holders
- Allows expressing intensity of preference

### 3. Proposal Types

- **Financial**: Treasury allocation, grants (standard voting only)
- **Sentiment**: Community preferences, non-financial direction
- **Upgrade**: Protocol upgrades
- **ParameterChange**: Configuration updates

### 4. Vote Tallying

**Standard Mode:**
- Voter's full voting weight applied to their choice
- Simple stake-weighted voting

**Quadratic Mode:**
- Voter specifies voice credits to spend
- Actual votes = integer_sqrt(credits)
- Must not exceed voter's available weight

**Delegation Handling:**
- Delegates receive aggregated weight from all delegators
- Direct votes use voter's own weight only
- No double-counting (voted addresses excluded from delegation tallies)

## Architecture

### Data Structures

```rust
pub enum VotingMode {
    Standard,   // Full weight voting
    Quadratic,  // Quadratic cost voting
}

pub enum ProposalType {
    Financial,
    Sentiment,
    Upgrade,
    ParameterChange,
}

pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: String,
    pub description: String,
    pub proposal_type: ProposalType,
    pub voting_mode: VotingMode,
    pub votes_for: i128,
    pub votes_against: i128,
    pub votes_abstain: i128,
    pub created_at: u64,
    pub expires_at: u64,
    pub executed: bool,
    pub quorum_required: i128,
}

pub struct VoteRecord {
    pub voter: Address,
    pub choice: VoteChoice,
    pub weight: i128,
    pub voice_credits_spent: i128,
    pub voted_at: u64,
}
```

### Storage Keys

- `Admin`: Admin address
- `VotingWeight(Address)`: Stake/weight for address (fallback, used only when vesting not configured)
- `Vesting`: Address of the Vesting contract (set by admin via `set_vesting_contract`)
- `Delegation(Address)`: Delegator → delegate mapping
- `DelegatedBy(Address)`: Reverse index for efficient weight calculation
- `Proposal(u64)`: Proposal data by ID
- `VoteRecord(u64, Address)`: Vote records per proposal per voter

## API Reference

### Initialization

```rust
pub fn initialize(
    env: Env,
    admin: Address,
    access_control: Address,
) -> Result<(), GovernanceError>
```

Initialize the contract with admin and access control addresses.

### Weight Management

```rust
pub fn set_voting_weight(
    env: Env,
    admin: Address,
    address: Address,
    weight: i128,
) -> Result<(), GovernanceError>
```

Set voting weight for an address (admin only). Used as fallback when no vesting
contract is configured. When a vesting contract is set, weight is computed from
the vested balance automatically and manual weights are ignored.

```rust
pub fn set_vesting_contract(
    env: Env,
    admin: Address,
    vesting_address: Address,
) -> Result<(), GovernanceError>
```

Set the vesting contract address (admin only). Once set, vote weight is computed
from the beneficiary's vested balance at the proposal's `created_at` snapshot
timestamp. Requires `admin.require_auth()`.

```rust
pub fn get_vesting_contract(env: Env) -> Option<Address>
```

Get the current vesting contract address, or `None` if not configured.

```rust
pub fn get_voting_weight(env: Env, address: Address) -> i128
```

Get total voting weight (own weight + delegated weight if delegate). When a
vesting contract is configured, own weight is the beneficiary's vested balance
at the current ledger timestamp.

### Delegation

```rust
pub fn delegate(
    env: Env,
    delegator: Address,
    delegate: Address,
) -> Result<(), GovernanceError>
```

Delegate voting weight to another address. Requires `delegator.require_auth()`.

```rust
pub fn revoke_delegation(
    env: Env,
    delegator: Address,
) -> Result<(), GovernanceError>
```

Revoke current delegation. Requires `delegator.require_auth()`.

```rust
pub fn get_delegate(env: Env, delegator: Address) -> Option<Address>
```

Get the current delegate for an address.

### Proposals

```rust
pub fn create_proposal(
    env: Env,
    proposer: Address,
    title: String,
    description: String,
    proposal_type: ProposalType,
    voting_mode: VotingMode,
    quorum_required: i128,
    duration: u64,  // 0 = use default (7 days)
) -> Result<u64, GovernanceError>
```

Create a new proposal. Returns proposal ID.

**Validation:**
- Financial proposals cannot use quadratic voting
- Duration defaults to 7 days if zero

```rust
pub fn get_proposal(env: Env, proposal_id: u64) -> Option<Proposal>
```

Retrieve proposal details.

### Voting

```rust
pub fn vote(
    env: Env,
    voter: Address,
    proposal_id: u64,
    choice: VoteChoice,
    voice_credits: i128,
) -> Result<(), GovernanceError>
```

Cast a vote on a proposal.

**Parameters:**
- `voice_credits`: For quadratic voting (ignored for standard voting)

**Rules:**
- Cannot vote twice on same proposal
- Must have voting weight > 0
- Proposal must not be expired or executed
- For quadratic: credits must be > 0 and ≤ voting weight
- Vesting contract must be set (fails closed with `VestingNotConfigured` if not)
- Weight is computed from the vesting contract at the snapshot timestamp (`proposal.created_at`)

```rust
pub fn get_vote_record(
    env: Env,
    proposal_id: u64,
    voter: Address,
) -> Option<VoteRecord>
```

Get vote record for a voter on a proposal.

### Execution

```rust
pub fn execute_proposal(
    env: Env,
    admin: Address,
    proposal_id: u64,
) -> Result<(), GovernanceError>
```

Execute a proposal (admin only). Requires quorum to be met.

## Edge Cases & Constraints

### Delegation

1. **Mid-vote revocation**: Affects only future votes, not past tallies
2. **Large delegate fan-in**: Delegated weight calculation is O(n) where n = number of delegators
   - Acceptable for reasonable delegate sizes (<100 delegators)
   - Gas cost scales linearly with delegator count
3. **Re-delegation**: Old delegate loses weight immediately
4. **Delegation without weight**: Allowed but contributes zero to delegate's weight

### Quadratic Voting

1. **Integer square-root**: Uses binary search algorithm
   - Always returns floor(sqrt(n))
   - No floating-point math, fully deterministic
2. **Sybil resistance limitation**: Splitting stake across addresses can game the system
   - Documented known limitation given on-chain identity constraints
   - Mitigated by proposal type restrictions (quadratic only for sentiment, not financial)
3. **Rounding consistency**: All voters use same integer sqrt implementation
4. **Credit validation**: Strictly enforced ≤ voting weight

### Vote Tallying

1. **Precedence rule**: Direct vote always counts, delegation ignored for that voter
2. **Double-counting prevention**: Voters who cast direct votes excluded from delegation aggregation
3. **Arithmetic safety**: All tallies use checked arithmetic with overflow protection

## Testing

Test coverage: **90%+**

### Delegation Tests

- ✅ Delegation and revocation
- ✅ Circular delegation prevention
- ✅ Direct vote precedence
- ✅ Mid-vote revocation
- ✅ Large delegate fan-in (50+ delegators)
- ✅ Re-delegation
- ✅ Self-delegation rejection

### Quadratic Voting Tests

- ✅ Cost calculation correctness
- ✅ Voice credits exceed weight rejection
- ✅ Zero/negative credits rejection
- ✅ Mode selection per proposal
- ✅ Integer sqrt comprehensive coverage
- ✅ Multiple voters aggregation

### General Tests

- ✅ Proposal creation and validation
- ✅ Vote record tracking
- ✅ Already-voted prevention
- ✅ Quorum enforcement
- ✅ Proposal execution
- ✅ Abstain votes

Run tests:

```bash
cargo test --package kora-governance-voting --lib
```

## Security Considerations

1. **Authorization**: All state-changing functions require `require_auth()`
2. **Overflow protection**: Checked arithmetic throughout
3. **Reentrancy**: The vote path makes a read-only cross-contract call to the
   vesting contract (`get_vested_balance_at`). The vesting contract is
   non-reentrant and the governance path only reads, so reentrancy is not a
   concern.
4. **Delegation circularity**: Prevented at delegation time
5. **Vote finality**: Cannot change vote once cast
6. **Admin privileges**: Limited to weight management, execution, and setting
   the vesting contract address
7. **Fallback on misconfiguration**: If the vesting contract address is not
   set, `vote()` falls back to manually set weights (backward compatible).
   Once set, only vested balances from the vesting contract are used.

## Integration Example

```rust
use kora_governance_voting::{
    GovernanceVotingContractClient,
    ProposalType,
    VotingMode,
    VoteChoice,
};
use kora_vesting::{
    VestingContractClient,
};

// 1. Deploy & initialize vesting contract
let vesting = VestingContractClient::new(&env, &vesting_addr);
vesting.initialize(&admin, &token_address);

// Create grant for alice: 10_000 tokens vesting over 1 year
vesting.create_grant(&admin, &alice, &10_000, &now, &604800, &31536000);

// 2. Deploy & initialize governance voting
let gov = GovernanceVotingContractClient::new(&env, &gov_contract_id);
gov.initialize(&admin, &access_control);

// 3. Wire vesting contract into governance for on-chain weight computation
gov.set_vesting_contract(&admin, &vesting_addr);

// 4. Alice delegates to Bob (delegates her vested balance)
gov.delegate(&alice, &bob);

// 5. Bob's weight = his own vested balance + Alice's vested balance
let bob_weight = gov.get_voting_weight(&bob); // computed from vesting contract

// 6. Create a sentiment proposal with quadratic voting
let proposal_id = gov.create_proposal(
    &proposer,
    &String::from_str(&env, "Protocol Direction"),
    &String::from_str(&env, "Should we..."),
    &ProposalType::Sentiment,
    &VotingMode::Quadratic,
    &100, // quorum
    &0,   // default duration
);

// 7. Bob votes with 49 voice credits → 7 votes (sqrt(49))
//    Weight is the vested balance at proposal.created_at snapshot
gov.vote(&bob, &proposal_id, &VoteChoice::For, &49);

// 8. Execute if quorum met
gov.execute_proposal(&admin, &proposal_id);
```

## Future Enhancements

- Multi-level delegation (with cycle detection)
- Delegated voting on behalf of delegators
- Time-weighted voting (longer stake lock = more weight)
- Vote history and analytics
- Snapshot-based voting (prevent flash loans)
- Proposal categories with different quorum rules

## License

MIT
