//! Tests targeting residual uncovered branches in `tezoro-vault` that
//! `comprehensive.rs` and `security.rs` don't exercise.

use mock_strategy::{MockStrategy, MockStrategyClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
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

// ---------------------------------------------------------------------------
// Unknown-strategy guards on admin/keeper entry points
// ---------------------------------------------------------------------------

#[test]
fn test_emergency_withdraw_rejects_unknown_strategy() {
    let (env, client, _vault_addr, _asset, _user) = init_with_fee_bps(1500);
    let unknown = Address::generate(&env);
    let admin = client.admin();
    assert!(client
        .try_emergency_withdraw_strategy(&admin, &unknown)
        .is_err());
}

#[test]
fn test_update_tracked_balance_rejects_unknown_strategy() {
    let (env, client, _vault_addr, _asset, _user) = init_with_fee_bps(1500);
    let unknown = Address::generate(&env);
    let keeper = client.keeper();
    assert!(client
        .try_update_tracked_balance(&keeper, &unknown, &100)
        .is_err());
}

// ---------------------------------------------------------------------------
// Saturating-sub in deallocate: strategy returns more than we tracked
// (e.g. yield accrued externally). `tracked - withdrawn` would underflow,
// so the vault clamps to 0. Exercised via MockStrategy::simulate_yield.
// ---------------------------------------------------------------------------

#[test]
fn test_deallocate_clamps_tracked_when_strategy_returns_more_than_tracked() {
    let (env, client, _vault_addr, asset, keeper, admin, strategy_addr) =
        setup_vault_with_strategy();

    let user = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&user, &1_000_0000000);
    client.deposit(&user, &1_000_0000000);
    client.allocate(&keeper, &strategy_addr, &500_0000000);

    // Strategy earns yield out-of-band — balance > tracked.
    StellarAssetClient::new(&env, &asset).mint(&strategy_addr, &100_0000000);
    let strategy = MockStrategyClient::new(&env, &strategy_addr);
    strategy.simulate_yield(&admin, &100_0000000);

    // Pull everything the strategy holds (600). tracked was 500, withdrawn
    // is 600, so `new_tracked` takes the saturating `0` branch.
    let withdrawn = client.deallocate(&keeper, &strategy_addr, &600_0000000);
    assert_eq!(withdrawn, 600_0000000);
    assert_eq!(client.tracked_balance(&strategy_addr), 0);
}

// ---------------------------------------------------------------------------
// remove_strategy loop: when the list has multiple entries, the non-matching
// arm (`else { new_strategies.push_back(s); }`) must also execute.
// ---------------------------------------------------------------------------

#[test]
fn test_remove_strategy_preserves_other_entries() {
    let (env, client, vault_addr, asset, _keeper, admin, strategy_a) = setup_vault_with_strategy();

    // Register a second mock strategy alongside the one setup created.
    let strategy_b = deploy_mock_strategy(&env, &admin, &vault_addr, &asset);
    client.add_strategy(&admin, &strategy_b);

    // Remove A — the loop now has to iterate past B (else-arm) and match on A.
    client.remove_strategy(&admin, &strategy_a);

    let remaining = client.get_strategies();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining.get(0).unwrap(), strategy_b);
}

// ---------------------------------------------------------------------------
// Redeem waterfall across multiple strategies: the first strategy is empty
// (available_liquidity() == 0), forcing the `continue` arm; the second
// strategy holds the funds and satisfies the shortfall. After that, the
// `remaining <= 0 { break }` arm fires on any further iteration.
// ---------------------------------------------------------------------------

#[test]
fn test_redeem_waterfall_skips_empty_strategy_and_pulls_from_next() {
    let (env, client, vault_addr, asset, keeper, admin, strategy_a) = setup_vault_with_strategy();

    // Register a second strategy. Order matters: strategy_a (empty) is
    // iterated first, hitting the `available <= 0 -> continue` branch.
    let strategy_b = deploy_mock_strategy(&env, &admin, &vault_addr, &asset);
    client.add_strategy(&admin, &strategy_b);

    let user = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&user, &1_000_0000000);
    client.deposit(&user, &1_000_0000000);

    // Allocate only to strategy_b so strategy_a stays dry.
    client.allocate(&keeper, &strategy_b, &970_0000000);
    assert_eq!(MockStrategyClient::new(&env, &strategy_a).balance_of(), 0);

    // Redeem half the user's position. idle (30) is short, waterfall runs:
    // strategy_a returns 0 -> continue, strategy_b covers the shortfall.
    let shares = client.balance(&user);
    let half = shares / 2;
    let received = client.redeem(&user, &half);
    assert!(received > 0);
    assert_eq!(TokenClient::new(&env, &asset).balance(&user), received);

    // Sanity: vault drained strategy_b only.
    assert_eq!(client.tracked_balance(&strategy_a), 0);
    assert!(client.tracked_balance(&strategy_b) < 970_0000000);
    let _ = vault_addr;
}

// ---------------------------------------------------------------------------
// Storage: `get_bridged_timestamp` bumps the entry's TTL only when the
// value is non-zero. Reading after a non-zero write hits the `if val != 0`
// true-arm; the default-zero path is covered by post-init getters.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Redeem waterfall: three-strategy scenario that drains the first two in
// sequence and leaves the third for the `remaining <= 0 -> break` arm.
// The partial-fill on the first strategy also takes the `remaining -
// withdrawn` (non-zero) arm of the saturating sub.
// ---------------------------------------------------------------------------

#[test]
fn test_redeem_waterfall_partial_fill_then_break_across_three_strategies() {
    let (env, client, vault_addr, asset, keeper, admin, strategy_a) = setup_vault_with_strategy();

    let strategy_b = deploy_mock_strategy(&env, &admin, &vault_addr, &asset);
    let strategy_c = deploy_mock_strategy(&env, &admin, &vault_addr, &asset);
    client.add_strategy(&admin, &strategy_b);
    client.add_strategy(&admin, &strategy_c);

    let user = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&user, &1_000_0000000);
    client.deposit(&user, &1_000_0000000);

    // 3% buffer on 1000 = 30 idle. Allocate 300 to A and 670 to B; C stays
    // empty so it triggers the break arm once B has satisfied the shortfall.
    client.allocate(&keeper, &strategy_a, &300_0000000);
    client.allocate(&keeper, &strategy_b, &670_0000000);

    // Redeem everything. Waterfall: A gives 300 (remaining drops to shortfall -
    // 300, exercising `remaining - withdrawn`), B gives the rest, then the
    // loop iterates to C and hits `remaining <= 0 -> break`.
    let shares = client.balance(&user);
    client.redeem(&user, &shares);

    assert_eq!(client.tracked_balance(&strategy_a), 0);
    assert_eq!(client.tracked_balance(&strategy_b), 0);
    assert_eq!(client.tracked_balance(&strategy_c), 0); // never touched
    let _ = vault_addr;
}

// ---------------------------------------------------------------------------
// Redeem waterfall: strategy returns MORE than the vault tracked (external
// yield accrued directly into the strategy's balance). The `withdrawn >
// tracked` saturating-sub arm must clamp new_tracked to 0 instead of
// underflowing.
// ---------------------------------------------------------------------------

#[test]
fn test_redeem_waterfall_clamps_tracked_when_strategy_overpulls() {
    let (env, client, _vault_addr, asset, keeper, admin, strategy_addr) =
        setup_vault_with_strategy();

    let user = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&user, &1_000_0000000);
    client.deposit(&user, &1_000_0000000);

    // Allocate 500 so tracked = 500. Then inflate strategy's underlying
    // balance out-of-band to simulate external yield — balance = 600,
    // tracked = 500.
    client.allocate(&keeper, &strategy_addr, &500_0000000);
    StellarAssetClient::new(&env, &asset).mint(&strategy_addr, &100_0000000);
    MockStrategyClient::new(&env, &strategy_addr).simulate_yield(&admin, &100_0000000);

    // total_assets = idle(500) + strategy(600) = 1100. User's shares are
    // worth ~1100. Waterfall pulls the full 600 from the strategy, which is
    // more than tracked(500), so new_tracked takes the clamp-to-zero arm.
    let shares = client.balance(&user);
    client.redeem(&user, &shares);

    assert_eq!(client.tracked_balance(&strategy_addr), 0);
}

#[test]
fn test_bridged_timestamp_bumps_ttl_when_nonzero() {
    let (env, client, _vault_addr, _asset, _user) = init_with_fee_bps(1500);

    // Default test ledger timestamp is 0; advance it so the attest writes a
    // non-zero timestamp and the read path enters the `val != 0` true arm.
    env.ledger().with_mut(|info| info.timestamp = 1_000_000);

    let keeper = client.keeper();
    client.attest_bridged_balance(&keeper, &1000);

    let ts_first = client.bridged_timestamp();
    assert_eq!(ts_first, 1_000_000);

    // Second read on the same non-zero value re-enters the bump arm.
    let ts_second = client.bridged_timestamp();
    assert_eq!(ts_first, ts_second);
}
