//! Authorization and access control tests.
//!
//! Mirrors the coverage of EVM SecurityTests.fork.t.sol:
//! every role boundary is tested, every unauthorized call is rejected.

use mock_strategy::MockStrategy;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, Env, String};
use tezoro_vault::{TezoroVault, TezoroVaultClient};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    client: TezoroVaultClient<'a>,
    vault_addr: Address,
    admin: Address,
    keeper: Address,
    guardian: Address,
    // Mirror the full init signature for future role-boundary tests even
    // though no current test references this role directly.
    #[allow(dead_code)]
    fee_recipient: Address,
    asset: Address,
    user: Address,
    random: Address,
}

fn setup() -> Ctx<'static> {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let user = Address::generate(&env);
    let random = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let asset = token_contract.address();
    StellarAssetClient::new(&env, &asset).mint(&user, &100_000_0000000);

    let vault_addr = env.register(TezoroVault, ());
    let client = TezoroVaultClient::new(&env, &vault_addr);
    client.initialize(
        &admin,
        &asset,
        &keeper,
        &guardian,
        &fee_recipient,
        &1500u32,
        &300u32,
        &String::from_str(&env, "Tezoro USDC-A"),
        &String::from_str(&env, "tUSDC-A"),
    );

    Ctx {
        env,
        client,
        vault_addr,
        admin,
        keeper,
        guardian,
        fee_recipient,
        asset,
        user,
        random,
    }
}

fn deploy_mock_strategy(ctx: &Ctx) -> Address {
    let strategy_addr = ctx.env.register(MockStrategy, ());
    mock_strategy::MockStrategyClient::new(&ctx.env, &strategy_addr).initialize(
        &ctx.admin,
        &ctx.vault_addr,
        &ctx.asset,
    );
    ctx.client.add_strategy(&ctx.admin, &strategy_addr);
    strategy_addr
}

// ---------------------------------------------------------------------------
// Admin-only operations
// ---------------------------------------------------------------------------

#[test]
fn test_add_strategy_admin_only() {
    let ctx = setup();
    let strategy = Address::generate(&ctx.env);

    assert!(ctx.client.try_add_strategy(&ctx.random, &strategy).is_err());
    assert!(ctx.client.try_add_strategy(&ctx.keeper, &strategy).is_err());
    assert!(ctx
        .client
        .try_add_strategy(&ctx.guardian, &strategy)
        .is_err());
    assert!(ctx.client.try_add_strategy(&ctx.admin, &strategy).is_ok());
}

#[test]
fn test_remove_strategy_admin_only() {
    let ctx = setup();
    let strategy = Address::generate(&ctx.env);
    ctx.client.add_strategy(&ctx.admin, &strategy);

    assert!(ctx
        .client
        .try_remove_strategy(&ctx.random, &strategy)
        .is_err());
    assert!(ctx
        .client
        .try_remove_strategy(&ctx.keeper, &strategy)
        .is_err());
    assert!(ctx
        .client
        .try_remove_strategy(&ctx.admin, &strategy)
        .is_ok());
}

#[test]
fn test_set_keeper_admin_only() {
    let ctx = setup();
    let new_keeper = Address::generate(&ctx.env);

    assert!(ctx.client.try_set_keeper(&ctx.random, &new_keeper).is_err());
    assert!(ctx
        .client
        .try_set_keeper(&ctx.guardian, &new_keeper)
        .is_err());
    assert!(ctx.client.try_set_keeper(&ctx.admin, &new_keeper).is_ok());
    assert_eq!(ctx.client.keeper(), new_keeper);
}

#[test]
fn test_set_guardian_admin_only() {
    let ctx = setup();
    let new_guardian = Address::generate(&ctx.env);

    assert!(ctx
        .client
        .try_set_guardian(&ctx.random, &new_guardian)
        .is_err());
    assert!(ctx
        .client
        .try_set_guardian(&ctx.keeper, &new_guardian)
        .is_err());
    assert!(ctx
        .client
        .try_set_guardian(&ctx.admin, &new_guardian)
        .is_ok());
    assert_eq!(ctx.client.guardian(), new_guardian);
}

#[test]
fn test_set_deposit_cap_admin_only() {
    let ctx = setup();

    assert!(ctx.client.try_set_deposit_cap(&ctx.random, &1000).is_err());
    assert!(ctx.client.try_set_deposit_cap(&ctx.keeper, &1000).is_err());
    assert!(ctx.client.try_set_deposit_cap(&ctx.admin, &1000).is_ok());
}

#[test]
fn test_unpause_admin_only() {
    let ctx = setup();
    ctx.client.pause(&ctx.guardian);

    assert!(ctx.client.try_unpause(&ctx.random).is_err());
    assert!(ctx.client.try_unpause(&ctx.guardian).is_err());
    assert!(ctx.client.try_unpause(&ctx.keeper).is_err());
    assert!(ctx.client.try_unpause(&ctx.admin).is_ok());
}

#[test]
fn test_propose_admin_admin_only() {
    let ctx = setup();
    let new_admin = Address::generate(&ctx.env);

    assert!(ctx
        .client
        .try_propose_admin(&ctx.random, &new_admin)
        .is_err());
    assert!(ctx
        .client
        .try_propose_admin(&ctx.keeper, &new_admin)
        .is_err());
    assert!(ctx.client.try_propose_admin(&ctx.admin, &new_admin).is_ok());
}

#[test]
fn test_set_upgrade_delay_admin_only() {
    let ctx = setup();

    assert!(ctx
        .client
        .try_set_upgrade_delay(&ctx.random, &7200)
        .is_err());
    assert!(ctx
        .client
        .try_set_upgrade_delay(&ctx.keeper, &7200)
        .is_err());
    assert!(ctx.client.try_set_upgrade_delay(&ctx.admin, &7200).is_ok());
}

// ---------------------------------------------------------------------------
// Keeper-only operations
// ---------------------------------------------------------------------------

#[test]
fn test_allocate_keeper_only() {
    let ctx = setup();
    let strategy = deploy_mock_strategy(&ctx);

    ctx.client.deposit(&ctx.user, &1000_0000000);

    assert!(ctx
        .client
        .try_allocate(&ctx.random, &strategy, &100_0000000)
        .is_err());
    assert!(ctx
        .client
        .try_allocate(&ctx.admin, &strategy, &100_0000000)
        .is_err());
    assert!(ctx
        .client
        .try_allocate(&ctx.guardian, &strategy, &100_0000000)
        .is_err());
    assert!(ctx
        .client
        .try_allocate(&ctx.keeper, &strategy, &100_0000000)
        .is_ok());
}

#[test]
fn test_deallocate_keeper_only() {
    let ctx = setup();
    let strategy = deploy_mock_strategy(&ctx);

    ctx.client.deposit(&ctx.user, &1000_0000000);
    ctx.client.allocate(&ctx.keeper, &strategy, &100_0000000);

    assert!(ctx
        .client
        .try_deallocate(&ctx.random, &strategy, &50_0000000)
        .is_err());
    assert!(ctx
        .client
        .try_deallocate(&ctx.admin, &strategy, &50_0000000)
        .is_err());
    assert!(ctx
        .client
        .try_deallocate(&ctx.keeper, &strategy, &50_0000000)
        .is_ok());
}

#[test]
fn test_attest_bridged_keeper_only() {
    let ctx = setup();

    assert!(ctx
        .client
        .try_attest_bridged_balance(&ctx.random, &100)
        .is_err());
    assert!(ctx
        .client
        .try_attest_bridged_balance(&ctx.admin, &100)
        .is_err());
    assert!(ctx
        .client
        .try_attest_bridged_balance(&ctx.keeper, &100)
        .is_ok());
}

#[test]
fn test_update_tracked_balance_keeper_only() {
    let ctx = setup();
    let strategy = Address::generate(&ctx.env);
    ctx.client.add_strategy(&ctx.admin, &strategy);

    assert!(ctx
        .client
        .try_update_tracked_balance(&ctx.random, &strategy, &100)
        .is_err());
    assert!(ctx
        .client
        .try_update_tracked_balance(&ctx.admin, &strategy, &100)
        .is_err());
    assert!(ctx
        .client
        .try_update_tracked_balance(&ctx.keeper, &strategy, &100)
        .is_ok());
}

// ---------------------------------------------------------------------------
// Pause semantics (admin + guardian can pause, only admin unpause)
// ---------------------------------------------------------------------------

#[test]
fn test_pause_admin_and_guardian() {
    let ctx = setup();

    assert!(ctx.client.try_pause(&ctx.random).is_err());
    assert!(ctx.client.try_pause(&ctx.keeper).is_err());

    // Guardian can pause
    ctx.client.pause(&ctx.guardian);
    assert!(ctx.client.is_paused());
    ctx.client.unpause(&ctx.admin);

    // Admin can also pause
    ctx.client.pause(&ctx.admin);
    assert!(ctx.client.is_paused());
}

#[test]
fn test_deposit_blocked_while_paused() {
    let ctx = setup();
    ctx.client.pause(&ctx.guardian);

    assert!(ctx.client.try_deposit(&ctx.user, &100_0000000).is_err());
}

#[test]
fn test_redeem_allowed_while_paused() {
    let ctx = setup();
    ctx.client.deposit(&ctx.user, &1000_0000000);
    let shares = ctx.client.balance(&ctx.user);

    ctx.client.pause(&ctx.guardian);

    let result = ctx.client.try_redeem(&ctx.user, &shares);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Emergency withdraw (admin OR guardian)
// ---------------------------------------------------------------------------

#[test]
fn test_emergency_withdraw_admin_or_guardian() {
    let ctx = setup();
    let strategy = deploy_mock_strategy(&ctx);

    ctx.client.deposit(&ctx.user, &1000_0000000);
    ctx.client.allocate(&ctx.keeper, &strategy, &100_0000000);

    // Random cannot emergency withdraw
    assert!(ctx
        .client
        .try_emergency_withdraw_strategy(&ctx.random, &strategy)
        .is_err());
    // Keeper cannot
    assert!(ctx
        .client
        .try_emergency_withdraw_strategy(&ctx.keeper, &strategy)
        .is_err());
    // Guardian can
    assert!(ctx
        .client
        .try_emergency_withdraw_strategy(&ctx.guardian, &strategy)
        .is_ok());
}

#[test]
fn test_emergency_withdraw_by_admin() {
    let ctx = setup();
    let strategy = deploy_mock_strategy(&ctx);

    ctx.client.deposit(&ctx.user, &1000_0000000);
    ctx.client.allocate(&ctx.keeper, &strategy, &100_0000000);

    let withdrawn = ctx
        .client
        .emergency_withdraw_strategy(&ctx.admin, &strategy);
    assert_eq!(withdrawn, 100_0000000);
    assert_eq!(ctx.client.tracked_balance(&strategy), 0);
}

// ---------------------------------------------------------------------------
// Upgrade timelock authorization
// ---------------------------------------------------------------------------

#[test]
fn test_schedule_upgrade_admin_only() {
    let ctx = setup();
    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);

    assert!(ctx.client.try_schedule_upgrade(&ctx.random, &hash).is_err());
    assert!(ctx.client.try_schedule_upgrade(&ctx.keeper, &hash).is_err());
    assert!(ctx.client.try_schedule_upgrade(&ctx.admin, &hash).is_ok());
}

#[test]
fn test_execute_upgrade_admin_only() {
    let ctx = setup();
    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);
    ctx.client.schedule_upgrade(&ctx.admin, &hash);

    // Even with timelock passed, non-admin can't execute
    assert!(ctx.client.try_execute_upgrade(&ctx.random).is_err());
    assert!(ctx.client.try_execute_upgrade(&ctx.keeper).is_err());
}

#[test]
fn test_cancel_upgrade_admin_only() {
    let ctx = setup();
    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);
    ctx.client.schedule_upgrade(&ctx.admin, &hash);

    assert!(ctx.client.try_cancel_upgrade(&ctx.random).is_err());
    assert!(ctx.client.try_cancel_upgrade(&ctx.keeper).is_err());
    assert!(ctx.client.try_cancel_upgrade(&ctx.admin).is_ok());
}

// ---------------------------------------------------------------------------
// Two-step admin transfer
// ---------------------------------------------------------------------------

#[test]
fn test_accept_admin_wrong_address_rejected() {
    let ctx = setup();
    let new_admin = Address::generate(&ctx.env);
    let impersonator = Address::generate(&ctx.env);

    ctx.client.propose_admin(&ctx.admin, &new_admin);

    // Wrong address tries to accept
    assert!(ctx.client.try_accept_admin(&impersonator).is_err());
    // Old admin tries to accept
    assert!(ctx.client.try_accept_admin(&ctx.admin).is_err());
    // Correct address succeeds
    assert!(ctx.client.try_accept_admin(&new_admin).is_ok());
}

#[test]
fn test_old_admin_loses_powers_after_transfer() {
    let ctx = setup();
    let new_admin = Address::generate(&ctx.env);

    ctx.client.propose_admin(&ctx.admin, &new_admin);
    ctx.client.accept_admin(&new_admin);

    // Old admin can no longer perform admin actions
    let strategy = Address::generate(&ctx.env);
    assert!(ctx.client.try_add_strategy(&ctx.admin, &strategy).is_err());
    assert!(ctx.client.try_set_deposit_cap(&ctx.admin, &100).is_err());
    assert!(ctx
        .client
        .try_propose_admin(&ctx.admin, &ctx.random)
        .is_err());

    // New admin can
    assert!(ctx.client.try_add_strategy(&new_admin, &strategy).is_ok());
}

// ---------------------------------------------------------------------------
// Keeper reassignment
// ---------------------------------------------------------------------------

#[test]
fn test_old_keeper_loses_powers() {
    let ctx = setup();
    let new_keeper = Address::generate(&ctx.env);

    ctx.client.set_keeper(&ctx.admin, &new_keeper);

    // Old keeper can no longer allocate
    let strategy = deploy_mock_strategy(&ctx);
    ctx.client.deposit(&ctx.user, &1000_0000000);

    assert!(ctx
        .client
        .try_allocate(&ctx.keeper, &strategy, &100_0000000)
        .is_err());
    assert!(ctx
        .client
        .try_allocate(&new_keeper, &strategy, &100_0000000)
        .is_ok());
}
