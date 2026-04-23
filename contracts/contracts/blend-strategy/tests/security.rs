//! Authorization and edge case tests for BlendStrategy.
//!
//! Unit tests that don't need the full Blend fixture.
//! Integration tests (with real Blend pool) are in the `integration_test` module in lib.rs.

use blend_strategy::{BlendStrategy, BlendStrategyClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, Env};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    client: BlendStrategyClient<'a>,
    admin: Address,
    vault: Address,
    asset: Address,
    random: Address,
}

fn setup() -> Ctx<'static> {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let blend_pool = Address::generate(&env);
    let random = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let asset = token_contract.address();
    StellarAssetClient::new(&env, &asset).mint(&vault, &100_000_0000000);

    let contract_addr = env.register(BlendStrategy, ());
    let client = BlendStrategyClient::new(&env, &contract_addr);
    client.initialize(&admin, &vault, &asset, &blend_pool);

    Ctx {
        env,
        client,
        admin,
        vault,
        asset,
        random,
    }
}

// ---------------------------------------------------------------------------
// Vault-only operations
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_vault_only() {
    let ctx = setup();

    assert!(ctx.client.try_deposit(&ctx.random, &1000_0000000).is_err());
    assert!(ctx.client.try_deposit(&ctx.admin, &1000_0000000).is_err());
    // Vault can deposit (will fail at Blend pool level since mock, but auth passes)
}

#[test]
fn test_withdraw_vault_only() {
    let ctx = setup();

    assert!(ctx.client.try_withdraw(&ctx.random, &1000_0000000).is_err());
    assert!(ctx.client.try_withdraw(&ctx.admin, &1000_0000000).is_err());
}

#[test]
fn test_emergency_withdraw_vault_or_admin() {
    let ctx = setup();

    // Random cannot
    assert!(ctx.client.try_emergency_withdraw(&ctx.random).is_err());

    // Vault can (returns 0 since no balance)
    let result = ctx.client.try_emergency_withdraw(&ctx.vault);
    assert!(result.is_ok());

    // Admin can
    let result = ctx.client.try_emergency_withdraw(&ctx.admin);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Admin-only operations
// ---------------------------------------------------------------------------

#[test]
fn test_pause_admin_only() {
    let ctx = setup();

    assert!(ctx.client.try_pause(&ctx.random).is_err());
    assert!(ctx.client.try_pause(&ctx.vault).is_err());
    assert!(ctx.client.try_pause(&ctx.admin).is_ok());
}

#[test]
fn test_unpause_admin_only() {
    let ctx = setup();
    ctx.client.pause(&ctx.admin);

    assert!(ctx.client.try_unpause(&ctx.random).is_err());
    assert!(ctx.client.try_unpause(&ctx.vault).is_err());
    assert!(ctx.client.try_unpause(&ctx.admin).is_ok());
}

#[test]
fn test_set_vault_admin_only() {
    let ctx = setup();
    let new_vault = Address::generate(&ctx.env);

    assert!(ctx.client.try_set_vault(&ctx.random, &new_vault).is_err());
    assert!(ctx.client.try_set_vault(&ctx.vault, &new_vault).is_err());
    assert!(ctx.client.try_set_vault(&ctx.admin, &new_vault).is_ok());
    assert_eq!(ctx.client.vault(), new_vault);
}

#[test]
fn test_set_max_utilization_admin_only() {
    let ctx = setup();

    assert!(ctx
        .client
        .try_set_max_utilization(&ctx.random, &8000u32)
        .is_err());
    assert!(ctx
        .client
        .try_set_max_utilization(&ctx.vault, &8000u32)
        .is_err());
    assert!(ctx
        .client
        .try_set_max_utilization(&ctx.admin, &8000u32)
        .is_ok());
}

#[test]
fn test_set_min_backstop_admin_only() {
    let ctx = setup();

    assert!(ctx
        .client
        .try_set_min_backstop_coverage(&ctx.random, &500u32)
        .is_err());
    assert!(ctx
        .client
        .try_set_min_backstop_coverage(&ctx.admin, &500u32)
        .is_ok());
}

#[test]
fn test_set_approval_buffer_admin_only() {
    let ctx = setup();

    assert!(ctx
        .client
        .try_set_approval_buffer(&ctx.random, &200u32)
        .is_err());
    assert!(ctx
        .client
        .try_set_approval_buffer(&ctx.admin, &200u32)
        .is_ok());
}

// ---------------------------------------------------------------------------
// Two-step admin transfer
// ---------------------------------------------------------------------------

#[test]
fn test_propose_admin_admin_only() {
    let ctx = setup();
    let new = Address::generate(&ctx.env);

    assert!(ctx.client.try_propose_admin(&ctx.random, &new).is_err());
    assert!(ctx.client.try_propose_admin(&ctx.vault, &new).is_err());
    assert!(ctx.client.try_propose_admin(&ctx.admin, &new).is_ok());
}

#[test]
fn test_accept_admin_correct_address_only() {
    let ctx = setup();
    let new = Address::generate(&ctx.env);
    ctx.client.propose_admin(&ctx.admin, &new);

    assert!(ctx.client.try_accept_admin(&ctx.random).is_err());
    assert!(ctx.client.try_accept_admin(&ctx.admin).is_err());
    assert!(ctx.client.try_accept_admin(&new).is_ok());
}

#[test]
fn test_accept_admin_without_proposal_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_accept_admin(&ctx.random).is_err());
}

#[test]
fn test_old_admin_loses_powers() {
    let ctx = setup();
    let new_admin = Address::generate(&ctx.env);

    ctx.client.propose_admin(&ctx.admin, &new_admin);
    ctx.client.accept_admin(&new_admin);

    // Old admin can no longer pause
    assert!(ctx.client.try_pause(&ctx.admin).is_err());
    // New admin can
    assert!(ctx.client.try_pause(&new_admin).is_ok());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_zero_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_deposit(&ctx.vault, &0).is_err());
}

#[test]
fn test_deposit_negative_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_deposit(&ctx.vault, &-1).is_err());
}

#[test]
fn test_withdraw_zero_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_withdraw(&ctx.vault, &0).is_err());
}

#[test]
fn test_withdraw_negative_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_withdraw(&ctx.vault, &-1).is_err());
}

#[test]
fn test_deposit_while_paused_rejected() {
    let ctx = setup();
    ctx.client.pause(&ctx.admin);

    assert!(ctx.client.try_deposit(&ctx.vault, &1000_0000000).is_err());
}

#[test]
fn test_max_utilization_above_10000_rejected() {
    let ctx = setup();
    assert!(ctx
        .client
        .try_set_max_utilization(&ctx.admin, &10001u32)
        .is_err());
}

#[test]
fn test_min_backstop_above_10000_rejected() {
    let ctx = setup();
    assert!(ctx
        .client
        .try_set_min_backstop_coverage(&ctx.admin, &10001u32)
        .is_err());
}

#[test]
fn test_double_initialize_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let pool = Address::generate(&env);

    let addr = env.register(BlendStrategy, ());
    let client = BlendStrategyClient::new(&env, &addr);

    client.initialize(&admin, &vault, &asset, &pool);
    assert!(client
        .try_initialize(&admin, &vault, &asset, &pool)
        .is_err());
}

// ---------------------------------------------------------------------------
// Upgrade timelock
// ---------------------------------------------------------------------------

#[test]
fn test_schedule_upgrade_admin_only() {
    let ctx = setup();
    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);

    assert!(ctx.client.try_schedule_upgrade(&ctx.random, &hash).is_err());
    assert!(ctx.client.try_schedule_upgrade(&ctx.vault, &hash).is_err());
    assert!(ctx.client.try_schedule_upgrade(&ctx.admin, &hash).is_ok());
}

#[test]
fn test_cancel_upgrade_admin_only() {
    let ctx = setup();
    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);
    ctx.client.schedule_upgrade(&ctx.admin, &hash);

    assert!(ctx.client.try_cancel_upgrade(&ctx.random).is_err());
    assert!(ctx.client.try_cancel_upgrade(&ctx.admin).is_ok());
}

#[test]
fn test_cancel_nonexistent_upgrade_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_cancel_upgrade(&ctx.admin).is_err());
}

#[test]
fn test_upgrade_delay_below_minimum_rejected() {
    let ctx = setup();
    // MIN_UPGRADE_DELAY = 3600
    assert!(ctx.client.try_set_upgrade_delay(&ctx.admin, &3599).is_err());
    assert!(ctx.client.try_set_upgrade_delay(&ctx.admin, &3600).is_ok());
}

// ---------------------------------------------------------------------------
// Harvest authorization
// ---------------------------------------------------------------------------

#[test]
fn test_harvest_vault_or_admin_only() {
    let ctx = setup();

    assert!(ctx.client.try_harvest(&ctx.random).is_err());
    // Vault and admin can call (will fail at pool level but auth passes)
}

// ---------------------------------------------------------------------------
// View functions on fresh state
// ---------------------------------------------------------------------------

#[test]
fn test_initial_state() {
    let ctx = setup();

    assert_eq!(ctx.client.balance_of(), 0);
    assert!(!ctx.client.is_paused());
    assert_eq!(ctx.client.asset(), ctx.asset);
    assert_eq!(ctx.client.vault(), ctx.vault);
}
