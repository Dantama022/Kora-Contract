//! Structured fuzzing for secondary-market buy and cancel settlement.
//!
//! Successful buys must debit the buyer by the listing price, pay the exact
//! protocol fee to the treasury, pay the remainder to the seller, and transfer
//! position ownership without changing its size. Cancels must remove the
//! listing without moving tokens or changing position ownership.

use arbitrary::Arbitrary;
use kora_secondary_market::{SecondaryMarket, SecondaryMarketClient};
use kora_shared::types::{Pool, Position};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    testutils::{Address as _, EnvTestConfig},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

use crate::gen;

const MAX_TRADE_PRICE: i128 = 1_000_000_000_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MockPoolError {
    NotFound = 1,
}

#[contracttype]
#[derive(Clone)]
enum MockPoolKey {
    Pool(u64),
    Position(u64, Address),
}

#[contract]
pub struct SettlementPool;

#[contractimpl]
impl SettlementPool {
    pub fn configure(env: Env, pool: Pool, position: Position) {
        env.storage()
            .persistent()
            .set(&MockPoolKey::Pool(pool.invoice_id), &pool);
        env.storage().persistent().set(
            &MockPoolKey::Position(position.invoice_id, position.investor.clone()),
            &position,
        );
    }

    pub fn get_pool(env: Env, invoice_id: u64) -> Result<Pool, MockPoolError> {
        env.storage()
            .persistent()
            .get(&MockPoolKey::Pool(invoice_id))
            .ok_or(MockPoolError::NotFound)
    }

    pub fn get_position(env: Env, invoice_id: u64, investor: Address) -> Position {
        env.storage()
            .persistent()
            .get(&MockPoolKey::Position(invoice_id, investor.clone()))
            .unwrap_or(Position {
                investor,
                invoice_id,
                contributed: 0,
                share_bps: 0,
                yield_claimed: 0,
            })
    }

    pub fn transfer_position(
        env: Env,
        invoice_id: u64,
        from: Address,
        to: Address,
    ) -> Result<(), MockPoolError> {
        from.require_auth();
        let key = MockPoolKey::Position(invoice_id, from.clone());
        let mut position: Position = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(MockPoolError::NotFound)?;
        env.storage().persistent().remove(&key);
        position.investor = to.clone();
        env.storage()
            .persistent()
            .set(&MockPoolKey::Position(invoice_id, to), &position);
        Ok(())
    }

    pub fn owns(env: Env, invoice_id: u64, investor: Address) -> bool {
        env.storage()
            .persistent()
            .has(&MockPoolKey::Position(invoice_id, investor))
    }
}

#[derive(Arbitrary, Debug)]
enum Action {
    Buy,
    Cancel,
}

#[derive(Arbitrary, Debug)]
struct SettlementCase {
    action: Action,
    price_raw: i128,
    price_tag: u8,
    fee_raw: u32,
    fee_tag: u8,
    position_raw: i128,
    position_tag: u8,
}

fn positive_bounded_amount(raw: i128) -> i128 {
    (raw.unsigned_abs() % MAX_TRADE_PRICE as u128) as i128 + 1
}

fn trade_price(raw: i128, tag: u8) -> i128 {
    match tag % 5 {
        0 => 0,
        1 => 1,
        2 => MAX_TRADE_PRICE,
        _ => positive_bounded_amount(raw),
    }
}

fn fee_rate(raw: u32, tag: u8) -> u32 {
    match tag % 6 {
        0 => 0,
        1 => 1,
        2 => 1_000,
        3 => 1_001,
        4 => u32::MAX,
        _ => gen::small_u32(raw, tag),
    }
}

fn position_size(raw: i128, tag: u8) -> i128 {
    match tag % 4 {
        0 => 0,
        1 => 1,
        2 => i128::MAX / 2,
        _ => positive_bounded_amount(raw),
    }
}

pub fn run(data: &[u8]) {
    let mut env = Env::default();
    env.set_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool_id = env.register_contract(None, SettlementPool);
    let pool = SettlementPoolClient::new(&env, &pool_id);
    let token_client = TokenClient::new(&env, &token);
    let token_admin = StellarAssetClient::new(&env, &token);
    let mut case_index = 0u64;

    super::drive::<SettlementCase, _>(data, |case| {
        let invoice_id = 1u64.saturating_add(case_index);
        case_index = case_index.saturating_add(1);

        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);
        let retry_buyer = Address::generate(&env);
        let price = trade_price(case.price_raw, case.price_tag);
        let fee_bps = fee_rate(case.fee_raw, case.fee_tag);
        let contributed = position_size(case.position_raw, case.position_tag);

        let pool_state = Pool {
            invoice_id,
            token: token.clone(),
            total_funded: contributed,
            face_value: contributed,
            repaid_amount: 0,
            is_closed: false,
            late_penalty_bps: 0,
            total_owed: contributed,
            penalty_applied: false,
        };
        let position = Position {
            investor: seller.clone(),
            invoice_id,
            contributed,
            share_bps: 10_000,
            yield_claimed: 0,
        };
        pool.configure(&pool_state, &position);

        let market_id = env.register_contract(None, SecondaryMarket);
        let market = SecondaryMarketClient::new(&env, &market_id);
        let _ = market.try_initialize(&admin, &pool_id, &treasury, &fee_bps);

        if price > 0 {
            token_admin.mint(&buyer, &price);
            token_admin.mint(&retry_buyer, &price);
        }
        let buyer_before = token_client.balance(&buyer);
        let seller_before = token_client.balance(&seller);
        let treasury_before = token_client.balance(&treasury);

        let listed = market.try_list_position(&seller, &invoice_id, &token, &price);
        if !matches!(listed, Ok(Ok(()))) {
            return;
        }

        match case.action {
            Action::Buy => {
                let bought = market.try_buy_position(&buyer, &invoice_id, &seller);
                if !matches!(bought, Ok(Ok(()))) {
                    return;
                }

                let expected_fee = price * fee_bps as i128 / 10_000;
                let expected_proceeds = price - expected_fee;
                assert_eq!(buyer_before - token_client.balance(&buyer), price);
                assert_eq!(token_client.balance(&seller) - seller_before, expected_proceeds);
                assert_eq!(token_client.balance(&treasury) - treasury_before, expected_fee);
                assert!(!pool.owns(&invoice_id, &seller));
                assert!(pool.owns(&invoice_id, &buyer));
                let moved = pool.get_position(&invoice_id, &buyer);
                assert_eq!(moved.investor, buyer);
                assert_eq!(moved.contributed, contributed);
                assert_eq!(moved.share_bps, 10_000);

                let buyer_retry_before = token_client.balance(&retry_buyer);
                assert!(market
                    .try_buy_position(&retry_buyer, &invoice_id, &seller)
                    .is_ok_and(|result| result.is_err()));
                assert_eq!(token_client.balance(&retry_buyer), buyer_retry_before);
            }
            Action::Cancel => {
                let cancelled = market.try_cancel_listing(&seller, &invoice_id);
                if !matches!(cancelled, Ok(Ok(()))) {
                    return;
                }
                assert_eq!(token_client.balance(&buyer), buyer_before);
                assert_eq!(token_client.balance(&seller), seller_before);
                assert_eq!(token_client.balance(&treasury), treasury_before);
                assert!(pool.owns(&invoice_id, &seller));
                assert!(!pool.owns(&invoice_id, &buyer));
                assert!(market
                    .try_cancel_listing(&seller, &invoice_id)
                    .is_ok_and(|result| result.is_err()));
            }
        }
    });
}