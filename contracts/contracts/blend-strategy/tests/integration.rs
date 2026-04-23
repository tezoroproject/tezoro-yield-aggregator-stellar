//! Integration tests for BlendStrategy against a real Blend pool deployed
//! via `BlendFixture`. Exercises happy paths that `tests/security.rs`
//! cannot reach (deposit/withdraw/emergency_withdraw/harvest/view funcs
//! with a live reserve).

use blend_contract_sdk::pool;
use blend_contract_sdk::testutils::{default_reserve_config, BlendFixture};
use blend_strategy::{BlendStrategy, BlendStrategyClient};
use soroban_sdk::testutils::{Address as _, BytesN as _};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, BytesN, Env, String};

// ---------------------------------------------------------------------------
// Shared setup
// ---------------------------------------------------------------------------

struct Fixture<'a> {
    env: Env,
    strategy: BlendStrategyClient<'a>,
    strategy_addr: Address,
    admin: Address,
    vault: Address,
    usdc: Address,
}

fn setup_with_pool() -> Fixture<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let deployer = Address::generate(&env);
    let admin = Address::generate(&env);
    let vault = Address::generate(&env);

    let blnd = env
        .register_stellar_asset_contract_v2(deployer.clone())
        .address();
    let usdc = env
        .register_stellar_asset_contract_v2(deployer.clone())
        .address();

    let blend = BlendFixture::deploy(&env, &deployer, &blnd, &usdc);

    // Deploy a pool with a dummy oracle — supply/withdraw of collateral
    // without borrowing does not exercise the oracle, so this is safe.
    let oracle = Address::generate(&env);
    let pool_addr = blend.pool_factory.deploy(
        &deployer,
        &String::from_str(&env, "test-pool"),
        &BytesN::<32>::random(&env),
        &oracle,
        &1_000_000, // 10% take rate (Blend SCALAR_7: 0.1 * 1e7)
        &4,
        &1_0000000,
    );
    let pool_client = pool::Client::new(&env, &pool_addr);
    let reserve_config = default_reserve_config();
    pool_client.queue_set_reserve(&usdc, &reserve_config);
    pool_client.set_reserve(&usdc);

    blend
        .backstop
        .deposit(&deployer, &pool_addr, &50_000_0000000);
    pool_client.set_status(&3);
    pool_client.update_status();

    // Mint USDC to the vault
    StellarAssetClient::new(&env, &usdc).mint(&vault, &1_000_000_0000000);

    // Deploy strategy against this real pool
    let strategy_addr = env.register(BlendStrategy, ());
    let strategy = BlendStrategyClient::new(&env, &strategy_addr);
    strategy.initialize(&admin, &vault, &usdc, &pool_addr);

    env.cost_estimate().budget().reset_default();

    Fixture {
        env,
        strategy,
        strategy_addr,
        admin,
        vault,
        usdc,
    }
}

fn fund_and_deposit(fx: &Fixture, amount: i128) {
    TokenClient::new(&fx.env, &fx.usdc).transfer(&fx.vault, &fx.strategy_addr, &amount);
    fx.strategy.deposit(&fx.vault, &amount);
}

// ---------------------------------------------------------------------------
// Deposit / withdraw happy paths
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_updates_tracked_balance() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 100_0000000);
    assert_eq!(fx.strategy.balance_of(), 100_0000000);
}

#[test]
fn test_multiple_deposits_accumulate() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 100_0000000);
    fund_and_deposit(&fx, 50_0000000);
    assert_eq!(fx.strategy.balance_of(), 150_0000000);
}

#[test]
fn test_partial_withdraw_reduces_balance() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 200_0000000);
    let withdrawn = fx.strategy.withdraw(&fx.vault, &50_0000000);
    assert_eq!(withdrawn, 50_0000000);
    assert_eq!(fx.strategy.balance_of(), 150_0000000);
}

#[test]
fn test_withdraw_more_than_tracked_clamps() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 100_0000000);
    let withdrawn = fx.strategy.withdraw(&fx.vault, &500_0000000);
    assert_eq!(withdrawn, 100_0000000);
    assert_eq!(fx.strategy.balance_of(), 0);
}

#[test]
fn test_withdraw_with_zero_balance_rejected() {
    let fx = setup_with_pool();
    let result = fx.strategy.try_withdraw(&fx.vault, &100_0000000);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Emergency withdraw happy paths
// ---------------------------------------------------------------------------

#[test]
fn test_emergency_withdraw_by_admin_pulls_all() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 300_0000000);

    let before = TokenClient::new(&fx.env, &fx.usdc).balance(&fx.vault);
    let withdrawn = fx.strategy.emergency_withdraw(&fx.admin);
    assert!(withdrawn > 0);
    assert_eq!(fx.strategy.balance_of(), 0);
    let after = TokenClient::new(&fx.env, &fx.usdc).balance(&fx.vault);
    assert_eq!(after - before, withdrawn);
}

#[test]
fn test_emergency_withdraw_by_vault_pulls_all() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 150_0000000);
    let withdrawn = fx.strategy.emergency_withdraw(&fx.vault);
    assert!(withdrawn > 0);
    assert_eq!(fx.strategy.balance_of(), 0);
}

#[test]
fn test_emergency_withdraw_empty_returns_zero() {
    let fx = setup_with_pool();
    let withdrawn = fx.strategy.emergency_withdraw(&fx.admin);
    assert_eq!(withdrawn, 0);
}

// ---------------------------------------------------------------------------
// View functions with real reserve
// ---------------------------------------------------------------------------

#[test]
fn test_available_liquidity_zero_when_empty() {
    let fx = setup_with_pool();
    assert_eq!(fx.strategy.available_liquidity(), 0);
}

#[test]
fn test_available_liquidity_bounded_by_tracked_balance() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 100_0000000);
    let available = fx.strategy.available_liquidity();
    // With no borrowing, pool_available is large; tracked is the limit
    assert!(available <= 100_0000000);
    assert!(available >= 0);
}

#[test]
fn test_is_healthy_empty_pool_passes() {
    let fx = setup_with_pool();
    // Fresh pool with backstop seeded and no utilization
    assert!(fx.strategy.is_healthy());
}

#[test]
fn test_is_healthy_tight_thresholds_still_pass_on_idle_pool() {
    let fx = setup_with_pool();
    // Even with strict thresholds, an idle pool should be healthy
    fx.strategy.set_max_utilization(&fx.admin, &100u32); // 1%
    assert!(fx.strategy.is_healthy());
}

// ---------------------------------------------------------------------------
// Harvest happy path (no emissions configured -> returns 0)
// ---------------------------------------------------------------------------

#[test]
fn test_harvest_by_vault_returns_non_negative() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 100_0000000);
    let claimed = fx.strategy.harvest(&fx.vault);
    assert!(claimed >= 0);
}

#[test]
fn test_harvest_by_admin_allowed() {
    let fx = setup_with_pool();
    fund_and_deposit(&fx, 100_0000000);
    let claimed = fx.strategy.harvest(&fx.admin);
    assert!(claimed >= 0);
}

// ---------------------------------------------------------------------------
// Full deposit -> withdraw round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_full_round_trip() {
    let fx = setup_with_pool();
    let amount = 500_0000000;
    fund_and_deposit(&fx, amount);

    let vault_before = TokenClient::new(&fx.env, &fx.usdc).balance(&fx.vault);
    let withdrawn = fx.strategy.withdraw(&fx.vault, &amount);
    let vault_after = TokenClient::new(&fx.env, &fx.usdc).balance(&fx.vault);

    assert_eq!(withdrawn, amount);
    assert_eq!(vault_after - vault_before, amount);
    assert_eq!(fx.strategy.balance_of(), 0);
}
