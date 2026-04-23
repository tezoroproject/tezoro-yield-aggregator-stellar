//! Tests targeting residual uncovered branches in `tezoro-vault` that
//! `comprehensive.rs` and `security.rs` don't exercise.

use mock_strategy::{MockStrategy, MockStrategyClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env, String};
use tezoro_vault::{TezoroVault, TezoroVaultClient};

fn init_with_fee_bps(fee_bps: u32) -> (Env, TezoroVaultClient<'static>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &asset);

    let vault_addr = env.register(TezoroVault, ());
    let client = TezoroVaultClient::new(&env, &vault_addr);
    client.initialize(
        &admin,
        &asset,
        &keeper,
        &guardian,
        &fee_recipient,
        &fee_bps,
        &300u32,
        &String::from_str(&env, "Tezoro Z"),
        &String::from_str(&env, "tZ"),
    );

    let user = Address::generate(&env);
    sac.mint(&user, &10_000_0000000);

    (env, client, vault_addr, asset, user)
}

// ---------------------------------------------------------------------------
// collect_fees branches
// ---------------------------------------------------------------------------

#[test]
fn test_collect_fees_with_zero_fee_bps_rejected() {
    // Yield exists but fee_bps == 0 → the `fee_bps == 0` guard trips.
    let (env, client, vault_addr, asset, user) = init_with_fee_bps(0);
    client.deposit(&user, &10_000_0000000);

    // Simulate yield by minting directly into the vault.
    StellarAssetClient::new(&env, &asset).mint(&vault_addr, &1_000_0000000);

    assert!(client.try_collect_fees().is_err());
}

// ---------------------------------------------------------------------------
// Guardian-side branches on admin-or-guardian checks
// ---------------------------------------------------------------------------

#[test]
fn test_pause_rejected_for_random_caller() {
    // `caller != admin && caller != guardian` — random caller exercises both
    // short-circuit branches.
    let (env, client, _vault_addr, _asset, _user) = init_with_fee_bps(1500);
    let random = Address::generate(&env);
    assert!(client.try_pause(&random).is_err());
}

#[test]
fn test_set_deposit_cap_rejected_for_keeper_caller() {
    // set_deposit_cap uses require_admin, but test the admin-only rejection
    // from a caller that passes require_auth (since mock_all_auths is on).
    let (env, client, _vault_addr, _asset, _user) = init_with_fee_bps(1500);
    let random = Address::generate(&env);
    assert!(client.try_set_deposit_cap(&random, &100).is_err());
}

// ---------------------------------------------------------------------------
// Strategy health-gate + partial deallocate
// ---------------------------------------------------------------------------

fn deploy_mock_strategy(
    env: &Env,
    admin: &Address,
    vault_addr: &Address,
    asset: &Address,
) -> Address {
    let strategy_addr = env.register(MockStrategy, ());
    let strategy = MockStrategyClient::new(env, &strategy_addr);
    strategy.initialize(admin, vault_addr, asset);
    strategy_addr
}

fn setup_vault_with_strategy() -> (
    Env,
    TezoroVaultClient<'static>,
    Address,
    Address,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    StellarAssetClient::new(&env, &asset).mint(&Address::generate(&env), &0); // warm the token

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
        &String::from_str(&env, "Tezoro Z"),
        &String::from_str(&env, "tZ"),
    );

    let strategy_addr = deploy_mock_strategy(&env, &admin, &vault_addr, &asset);
    client.add_strategy(&admin, &strategy_addr);

    (env, client, vault_addr, asset, keeper, admin, strategy_addr)
}

/// allocate() must gate on strategy.is_healthy(). Flipping the mock's health
/// flag off is the only way to exercise this branch without a real Blend pool.
#[test]
fn test_allocate_blocked_when_strategy_unhealthy() {
    let (env, client, _vault_addr, asset, keeper, admin, strategy_addr) =
        setup_vault_with_strategy();
    let _ = _vault_addr; // not needed for this case

    let user = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&user, &1_000_0000000);
    client.deposit(&user, &1_000_0000000);

    let strategy = MockStrategyClient::new(&env, &strategy_addr);
    strategy.set_healthy(&admin, &false);

    // Would-be-legal allocation (stays above the 3% idle buffer) rejected
    // because the strategy reports itself unhealthy.
    assert!(client
        .try_allocate(&keeper, &strategy_addr, &500_0000000)
        .is_err());

    // Restoring health lets the allocation go through — confirms the gate
    // is the only blocker rather than some orthogonal failure.
    strategy.set_healthy(&admin, &true);
    client.allocate(&keeper, &strategy_addr, &500_0000000);
    assert_eq!(client.tracked_balance(&strategy_addr), 500_0000000);
}

/// deallocate() must return exactly what the strategy had when the keeper
/// asks for more than the strategy can release. Mock returns min(amount,
/// balance), so the returned value is the strategy's full position and the
/// vault's tracked balance drops to zero.
#[test]
fn test_deallocate_partial_when_requesting_more_than_strategy_holds() {
    let (env, client, vault_addr, asset, keeper, _admin, strategy_addr) =
        setup_vault_with_strategy();

    let user = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&user, &1_000_0000000);
    client.deposit(&user, &1_000_0000000);

    // 3% idle buffer on 1000 = 30; allocate 970 so we leave exactly the buffer.
    client.allocate(&keeper, &strategy_addr, &970_0000000);
    assert_eq!(client.tracked_balance(&strategy_addr), 970_0000000);

    // Keeper requests 2x what's there. Strategy returns what it has; vault's
    // tracked balance must reconcile to 0 (withdrawn == tracked hits the
    // zero-out path in the `withdrawn > tracked ? 0 : tracked - withdrawn`
    // branch).
    let withdrawn = client.deallocate(&keeper, &strategy_addr, &2_000_0000000);
    assert_eq!(withdrawn, 970_0000000);
    assert_eq!(client.tracked_balance(&strategy_addr), 0);

    // Strategy should now be drained.
    let strategy = MockStrategyClient::new(&env, &strategy_addr);
    assert_eq!(strategy.balance_of(), 0);

    // Total assets unchanged — funds moved back to idle in the vault.
    assert_eq!(client.total_assets(), 1_000_0000000);
    assert_eq!(
        TokenClient::new(&env, &asset).balance(&vault_addr),
        1_000_0000000
    );
}

// ---------------------------------------------------------------------------
// View-function coverage
// ---------------------------------------------------------------------------

/// Walks the full SEP-41-ish surface and every admin-side getter in one go.
/// Tarpaulin previously missed these because neither `comprehensive.rs` nor
/// `security.rs` called them — the existing suites focus on mutations and
/// role gating, not read paths.
#[test]
fn test_all_view_functions_return_initialized_values() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

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

    // SEP-41 surface
    assert_eq!(client.name(), String::from_str(&env, "Tezoro USDC-A"));
    assert_eq!(client.symbol(), String::from_str(&env, "tUSDC-A"));
    assert_eq!(client.decimals(), 7);
    assert_eq!(client.total_supply(), 0);
    assert_eq!(client.balance(&Address::generate(&env)), 0);

    // Role + config getters
    assert_eq!(client.asset(), asset);
    assert_eq!(client.admin(), admin);
    assert_eq!(client.keeper(), keeper);
    assert_eq!(client.guardian(), guardian);
    assert!(!client.is_paused());
    assert_eq!(client.pending_admin(), None);
    assert_eq!(client.deposit_cap(), 0);
    assert_eq!(client.performance_fee_bps(), 1500);

    // Cross-chain / strategy tracking
    assert_eq!(client.bridged_balance(), 0);
    assert_eq!(client.bridged_timestamp(), 0);
    assert_eq!(client.tracked_balance(&Address::generate(&env)), 0);

    // Upgrade timelock surface
    assert!(client.upgrade_delay() > 0);
    assert_eq!(client.scheduled_upgrade(), None);

    // HWM starts at 1:1 NAV precision
    assert!(client.high_water_mark() > 0);
}

// ---------------------------------------------------------------------------
// Storage default paths (getters hitting the `unwrap_or(default)` branch)
// ---------------------------------------------------------------------------

/// Reads every optional-with-default storage field on a vault immediately
/// after `initialize` to exercise the `.unwrap_or(...)` branches in
/// `src/storage.rs`. The initialize flow writes most keys explicitly, but
/// this test also calls the few read paths that surface defaults when a
/// field is left untouched (e.g. deposit_cap remains 0 by default).
#[test]
fn test_storage_defaults_reachable_post_init() {
    let (env, client, _vault_addr, _asset, _user) = init_with_fee_bps(0);

    // Explicitly read every default-returning getter once so tarpaulin sees
    // the `unwrap_or(...)` arms as covered.
    let _ = client.deposit_cap(); // default 0
    let _ = client.performance_fee_bps(); // default 0 here (fee_bps=0 in init)
    let _ = client.upgrade_delay(); // default DEFAULT_UPGRADE_DELAY
    let _ = client.scheduled_upgrade(); // default None
    let _ = client.is_paused(); // default false
    let _ = client.high_water_mark(); // default NAV_PRECISION

    // Sanity-check a couple of the defaults explicitly.
    assert_eq!(client.deposit_cap(), 0);
    assert_eq!(client.performance_fee_bps(), 0);
    assert!(!client.is_paused());

    let _ = env; // keep the env alive for the borrow checker
}
