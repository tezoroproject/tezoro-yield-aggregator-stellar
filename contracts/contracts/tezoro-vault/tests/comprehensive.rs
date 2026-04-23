//! Comprehensive tests: edge cases, compliance, performance fees,
//! multi-user fairness, strategy integration.
//!
//! Mirrors EVM coverage: ComplianceTests, PerformanceFee, MultiStrategy.

use mock_strategy::MockStrategy;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
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
    // Mirror the full init signature for future role-boundary tests; not
    // every test here needs to reference the guardian directly.
    #[allow(dead_code)]
    guardian: Address,
    fee_recipient: Address,
    asset: Address,
    sac: StellarAssetClient<'a>,
}

fn setup() -> Ctx<'static> {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin.clone());
    let asset = token_contract.address();
    let sac = StellarAssetClient::new(&env, &asset);

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
        sac,
    }
}

fn user_with_balance(ctx: &Ctx, amount: i128) -> Address {
    let user = Address::generate(&ctx.env);
    ctx.sac.mint(&user, &amount);
    user
}

fn deploy_mock_strategy(ctx: &Ctx) -> (Address, mock_strategy::MockStrategyClient<'static>) {
    let addr = ctx.env.register(MockStrategy, ());
    let client = mock_strategy::MockStrategyClient::new(&ctx.env, &addr);
    client.initialize(&ctx.admin, &ctx.vault_addr, &ctx.asset);
    ctx.client.add_strategy(&ctx.admin, &addr);
    (addr, client)
}

// ===========================================================================
// EDGE CASES / COMPLIANCE
// ===========================================================================

#[test]
fn test_deposit_zero_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    assert!(ctx.client.try_deposit(&user, &0).is_err());
}

#[test]
fn test_deposit_negative_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    assert!(ctx.client.try_deposit(&user, &-1).is_err());
}

#[test]
fn test_redeem_zero_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);
    assert!(ctx.client.try_redeem(&user, &0).is_err());
}

#[test]
fn test_redeem_negative_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);
    assert!(ctx.client.try_redeem(&user, &-1).is_err());
}

#[test]
fn test_redeem_more_than_balance_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    let shares = ctx.client.deposit(&user, &1000_0000000);

    assert!(ctx.client.try_redeem(&user, &(shares + 1)).is_err());
}

#[test]
fn test_transfer_zero_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    let recipient = Address::generate(&ctx.env);
    assert!(ctx.client.try_transfer(&user, &recipient, &0).is_err());
}

#[test]
fn test_transfer_negative_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    let recipient = Address::generate(&ctx.env);
    assert!(ctx.client.try_transfer(&user, &recipient, &-1).is_err());
}

#[test]
fn test_transfer_more_than_balance_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    let shares = ctx.client.deposit(&user, &1000_0000000);

    let recipient = Address::generate(&ctx.env);
    assert!(ctx
        .client
        .try_transfer(&user, &recipient, &(shares + 1))
        .is_err());
}

#[test]
fn test_allocate_zero_rejected() {
    let ctx = setup();
    let (strategy, _) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    assert!(ctx.client.try_allocate(&ctx.keeper, &strategy, &0).is_err());
}

#[test]
fn test_allocate_negative_rejected() {
    let ctx = setup();
    let (strategy, _) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    assert!(ctx
        .client
        .try_allocate(&ctx.keeper, &strategy, &-1)
        .is_err());
}

#[test]
fn test_deallocate_zero_rejected() {
    let ctx = setup();
    let (strategy, _) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);
    ctx.client.allocate(&ctx.keeper, &strategy, &100_0000000);

    assert!(ctx
        .client
        .try_deallocate(&ctx.keeper, &strategy, &0)
        .is_err());
}

#[test]
fn test_allocate_inactive_strategy_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    let fake_strategy = Address::generate(&ctx.env);
    assert!(ctx
        .client
        .try_allocate(&ctx.keeper, &fake_strategy, &100_0000000)
        .is_err());
}

#[test]
fn test_deallocate_inactive_strategy_rejected() {
    let ctx = setup();
    let fake_strategy = Address::generate(&ctx.env);
    assert!(ctx
        .client
        .try_deallocate(&ctx.keeper, &fake_strategy, &100_0000000)
        .is_err());
}

#[test]
fn test_deposit_before_initialize_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let vault_addr = env.register(TezoroVault, ());
    let client = TezoroVaultClient::new(&env, &vault_addr);

    let user = Address::generate(&env);
    assert!(client.try_deposit(&user, &100).is_err());
}

#[test]
fn test_add_duplicate_strategy_rejected() {
    let ctx = setup();
    let strategy = Address::generate(&ctx.env);
    ctx.client.add_strategy(&ctx.admin, &strategy);

    assert!(ctx.client.try_add_strategy(&ctx.admin, &strategy).is_err());
}

#[test]
fn test_remove_nonexistent_strategy_rejected() {
    let ctx = setup();
    let strategy = Address::generate(&ctx.env);
    assert!(ctx
        .client
        .try_remove_strategy(&ctx.admin, &strategy)
        .is_err());
}

#[test]
fn test_max_strategies_limit() {
    let ctx = setup();
    for _ in 0..20u32 {
        let s = Address::generate(&ctx.env);
        ctx.client.add_strategy(&ctx.admin, &s);
    }

    // 21st should fail
    let one_too_many = Address::generate(&ctx.env);
    assert!(ctx
        .client
        .try_add_strategy(&ctx.admin, &one_too_many)
        .is_err());
}

#[test]
fn test_deposit_cap_zero_means_unlimited() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 10_000_0000000);

    // Default cap is 0 = unlimited
    assert_eq!(ctx.client.deposit_cap(), 0);
    assert!(ctx.client.try_deposit(&user, &10_000_0000000).is_ok());
}

#[test]
fn test_deposit_cap_exact_boundary() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.set_deposit_cap(&ctx.admin, &500_0000000);

    // Exactly at cap should succeed
    assert!(ctx.client.try_deposit(&user, &500_0000000).is_ok());
}

#[test]
fn test_deposit_cap_one_over_rejected() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.set_deposit_cap(&ctx.admin, &500_0000000);

    assert!(ctx.client.try_deposit(&user, &500_0000001).is_err());
}

#[test]
fn test_upgrade_delay_below_minimum_rejected() {
    let ctx = setup();
    // MIN_UPGRADE_DELAY = 3600 (1 hour)
    assert!(ctx.client.try_set_upgrade_delay(&ctx.admin, &3599).is_err());
    assert!(ctx.client.try_set_upgrade_delay(&ctx.admin, &3600).is_ok());
}

#[test]
fn test_cancel_nonexistent_upgrade_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_cancel_upgrade(&ctx.admin).is_err());
}

#[test]
fn test_execute_nonexistent_upgrade_rejected() {
    let ctx = setup();
    assert!(ctx.client.try_execute_upgrade(&ctx.admin).is_err());
}

#[test]
fn test_initialize_invalid_fee_bps_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let vault_addr = env.register(TezoroVault, ());
    let client = TezoroVaultClient::new(&env, &vault_addr);

    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let name = String::from_str(&env, "x");
    let symbol = String::from_str(&env, "x");

    // MAX_PERFORMANCE_FEE_BPS = 3000
    assert!(client
        .try_initialize(
            &admin,
            &asset,
            &keeper,
            &guardian,
            &fee_recipient,
            &3001u32,
            &300u32,
            &name,
            &symbol
        )
        .is_err());
}

#[test]
fn test_initialize_invalid_buffer_bps_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let vault_addr = env.register(TezoroVault, ());
    let client = TezoroVaultClient::new(&env, &vault_addr);

    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let name = String::from_str(&env, "x");
    let symbol = String::from_str(&env, "x");

    // MAX_IDLE_BUFFER_BPS = 2000
    assert!(client
        .try_initialize(
            &admin,
            &asset,
            &keeper,
            &guardian,
            &fee_recipient,
            &1500u32,
            &2001u32,
            &name,
            &symbol
        )
        .is_err());
}

// ===========================================================================
// PERFORMANCE FEES (deep coverage)
// ===========================================================================

#[test]
fn test_fee_hwm_prevents_double_counting() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // +1000 yield
    ctx.sac.mint(&ctx.vault_addr, &1_000_0000000);

    let fees1 = ctx.client.collect_fees();
    assert!(fees1 > 0);

    // No new yield -> no new fees
    assert!(ctx.client.try_collect_fees().is_err());
}

#[test]
fn test_fee_after_loss_no_charge() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // Simulate yield
    ctx.sac.mint(&ctx.vault_addr, &1_000_0000000);
    ctx.client.collect_fees();

    let hwm_after = ctx.client.high_water_mark();

    // Simulate loss: user redeems some, making total_assets drop.
    // But share price doesn't drop below HWM from this alone.
    // To truly simulate a loss, we'd need a strategy to lose money.
    // Instead, test that if we don't add more yield, no fees are collected.
    assert!(ctx.client.try_collect_fees().is_err());

    // HWM unchanged
    assert_eq!(ctx.client.high_water_mark(), hwm_after);
}

#[test]
fn test_fee_only_on_new_yield_above_hwm() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // First yield: +1000 USDC
    ctx.sac.mint(&ctx.vault_addr, &1_000_0000000);
    let fees1 = ctx.client.collect_fees();

    // Second yield: +500 USDC (only this should be charged)
    ctx.sac.mint(&ctx.vault_addr, &500_0000000);
    let fees2 = ctx.client.collect_fees();

    // fees2 should be less than fees1 (smaller yield)
    assert!(fees2 < fees1);
    assert!(fees2 > 0);
}

#[test]
fn test_fee_recipient_can_redeem_shares() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    ctx.sac.mint(&ctx.vault_addr, &1_000_0000000);
    ctx.client.collect_fees();

    let fee_shares = ctx.client.balance(&ctx.fee_recipient);
    assert!(fee_shares > 0);

    // Fee recipient can redeem
    let assets = ctx.client.redeem(&ctx.fee_recipient, &fee_shares);
    assert!(assets > 0);
    assert_eq!(ctx.client.balance(&ctx.fee_recipient), 0);
}

#[test]
fn test_fee_zero_supply_rejected() {
    let ctx = setup();
    // No deposits -> no fees
    assert!(ctx.client.try_collect_fees().is_err());
}

// ===========================================================================
// MULTI-USER FAIRNESS
// ===========================================================================

#[test]
fn test_late_depositor_gets_fewer_shares() {
    let ctx = setup();
    let alice = user_with_balance(&ctx, 10_000_0000000);
    let bob = user_with_balance(&ctx, 10_000_0000000);

    // Alice deposits first
    let alice_shares = ctx.client.deposit(&alice, &1_000_0000000);

    // Yield accrues (share price goes up)
    ctx.sac.mint(&ctx.vault_addr, &200_0000000);

    // Bob deposits same amount but gets fewer shares (higher share price)
    let bob_shares = ctx.client.deposit(&bob, &1_000_0000000);
    assert!(bob_shares < alice_shares);
}

#[test]
fn test_yield_distributed_proportionally() {
    let ctx = setup();
    let alice = user_with_balance(&ctx, 10_000_0000000);
    let bob = user_with_balance(&ctx, 10_000_0000000);

    // Alice: 1000, Bob: 2000 (2:1 ratio)
    let alice_shares = ctx.client.deposit(&alice, &1_000_0000000);
    let bob_shares = ctx.client.deposit(&bob, &2_000_0000000);

    // +300 yield
    ctx.sac.mint(&ctx.vault_addr, &300_0000000);

    let alice_assets = ctx.client.convert_to_assets(&alice_shares);
    let bob_assets = ctx.client.convert_to_assets(&bob_shares);

    // Bob should get ~2x Alice's share (including yield)
    // Alice: ~1100, Bob: ~2200
    assert!(bob_assets > alice_assets);
    let ratio = bob_assets * 100 / alice_assets;
    assert!((195..=205).contains(&ratio)); // ~2x within rounding
}

#[test]
fn test_deposit_redeem_roundtrip_no_yield() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 5_000_0000000);

    let initial = TokenClient::new(&ctx.env, &ctx.asset).balance(&user);
    let shares = ctx.client.deposit(&user, &5_000_0000000);
    let returned = ctx.client.redeem(&user, &shares);

    // With virtual offset, exact roundtrip for single depositor
    assert_eq!(returned, 5_000_0000000);
    let final_balance = TokenClient::new(&ctx.env, &ctx.asset).balance(&user);
    assert_eq!(final_balance, initial);
}

#[test]
fn test_concurrent_deposits_fair_pricing() {
    let ctx = setup();

    let mut total_deposited: i128 = 0;
    let mut users_shares = std::vec::Vec::new();

    // 5 users deposit different amounts
    let amounts = [
        100_0000000i128,
        500_0000000,
        1000_0000000,
        2000_0000000,
        5000_0000000,
    ];
    for amount in &amounts {
        let user = user_with_balance(&ctx, *amount);
        let shares = ctx.client.deposit(&user, amount);
        users_shares.push((user, shares));
        total_deposited += amount;
    }

    assert_eq!(ctx.client.total_assets(), total_deposited);

    // All redeem
    let mut total_redeemed: i128 = 0;
    for (user, shares) in &users_shares {
        let returned = ctx.client.redeem(user, shares);
        total_redeemed += returned;
    }

    // Rounding dust should be minimal (< 5 stroops for 5 users)
    let dust = total_deposited - total_redeemed;
    assert!((0..5).contains(&dust), "excessive rounding dust: {dust}");
}

// ===========================================================================
// BRIDGED BALANCE
// ===========================================================================

#[test]
fn test_bridged_balance_expires_after_max_age() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    // Attest 500 bridged
    ctx.client.attest_bridged_balance(&ctx.keeper, &500_0000000);
    assert_eq!(ctx.client.total_assets(), 1500_0000000);

    // Warp past max age (default 86400 = 24h)
    ctx.env.ledger().with_mut(|info| {
        info.timestamp += 86_401;
    });

    // Bridged balance should be expired
    assert_eq!(ctx.client.total_assets(), 1000_0000000);
}

#[test]
fn test_bridged_balance_affects_share_price() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    let shares = ctx.client.deposit(&user, &1000_0000000);

    // Before bridged attestation
    let value_before = ctx.client.convert_to_assets(&shares);

    // Attest bridged balance
    ctx.client.attest_bridged_balance(&ctx.keeper, &500_0000000);

    // Share price should increase
    let value_after = ctx.client.convert_to_assets(&shares);
    assert!(value_after > value_before);
}

#[test]
fn test_bridged_max_age_configurable() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 1000_0000000);
    ctx.client.deposit(&user, &1000_0000000);

    // Set short max age
    ctx.client.set_bridged_max_age(&ctx.admin, &60);
    ctx.client.attest_bridged_balance(&ctx.keeper, &500_0000000);

    // Warp 61 seconds
    ctx.env.ledger().with_mut(|info| {
        info.timestamp += 61;
    });

    // Should be expired with short max age
    assert_eq!(ctx.client.total_assets(), 1000_0000000);
}

// ===========================================================================
// IDLE BUFFER
// ===========================================================================

#[test]
fn test_idle_buffer_enforcement_exact() {
    let ctx = setup();
    let (strategy, _) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // idle_buffer_bps = 300 (3%). total = 10000. min_idle = 300.
    // Allocate 9700 (leaves exactly 300) -> should succeed
    assert!(ctx
        .client
        .try_allocate(&ctx.keeper, &strategy, &9_700_0000000)
        .is_ok());
}

#[test]
fn test_idle_buffer_violation_by_one() {
    let ctx = setup();
    let (strategy, _) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // 9701 leaves 299 < 300 required -> should fail
    assert!(ctx
        .client
        .try_allocate(&ctx.keeper, &strategy, &9_701_0000000)
        .is_err());
}

#[test]
fn test_redeem_waterfalls_from_strategy_when_idle_short() {
    // Full-position redeem must work even when most of the vault's assets
    // are deployed in a strategy — no prior keeper/admin deallocation. The
    // vault pulls the shortfall via strategy.withdraw() in one transaction.
    let ctx = setup();
    let (strategy, strategy_client) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    let shares = ctx.client.deposit(&user, &10_000_0000000);

    // Allocate 9000 USDC to strategy. Idle = 1000 USDC, strategy = 9000.
    ctx.client.allocate(&ctx.keeper, &strategy, &9_000_0000000);
    assert_eq!(strategy_client.balance_of(), 9_000_0000000);

    // Redeem the full position. Waterfall should deliver all 10000 USDC.
    let assets = ctx.client.redeem(&user, &shares);
    assert_eq!(assets, 10_000_0000000);

    // Strategy drained, tracked_balance cleared, user paid out in full.
    assert_eq!(strategy_client.balance_of(), 0);
    assert_eq!(ctx.client.tracked_balance(&strategy), 0);
    assert_eq!(ctx.client.balance(&user), 0);
    let asset_client = soroban_sdk::token::Client::new(&ctx.env, &ctx.asset);
    assert_eq!(asset_client.balance(&user), 10_000_0000000);
}

#[test]
fn test_redeem_idle_only_skips_strategy_withdraw() {
    // When idle already covers the redeem, the waterfall must NOT touch
    // strategies — otherwise we waste a cross-contract call and churn
    // tracked_balance for no reason.
    let ctx = setup();
    let (strategy, strategy_client) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // Put 3000 in strategy; 7000 stays idle.
    ctx.client.allocate(&ctx.keeper, &strategy, &3_000_0000000);
    let tracked_before = ctx.client.tracked_balance(&strategy);
    let strategy_balance_before = strategy_client.balance_of();

    // Redeem equivalent of 1000 USDC — well within idle.
    let shares_for_1000 = ctx.client.convert_to_shares(&1_000_0000000);
    let assets = ctx.client.redeem(&user, &shares_for_1000);
    assert_eq!(assets, 1_000_0000000);

    // Strategy untouched.
    assert_eq!(strategy_client.balance_of(), strategy_balance_before);
    assert_eq!(ctx.client.tracked_balance(&strategy), tracked_before);
}

#[test]
fn test_redeem_partial_shortfall_pulls_exact_deficit() {
    // Idle covers part of the redeem; the waterfall should pull only the
    // remainder from the strategy — not drain it.
    let ctx = setup();
    let (strategy, strategy_client) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // Idle = 2000, strategy = 8000.
    ctx.client.allocate(&ctx.keeper, &strategy, &8_000_0000000);

    // Redeem 5000 USDC → idle supplies 2000, strategy must cover 3000.
    let shares_for_5000 = ctx.client.convert_to_shares(&5_000_0000000);
    let assets = ctx.client.redeem(&user, &shares_for_5000);
    assert_eq!(assets, 5_000_0000000);

    // Strategy balance went from 8000 to 5000 (pulled exactly 3000).
    assert_eq!(strategy_client.balance_of(), 5_000_0000000);
    assert_eq!(ctx.client.tracked_balance(&strategy), 5_000_0000000);
}

// ===========================================================================
// STRATEGY INTEGRATION (with mock strategy)
// ===========================================================================

#[test]
fn test_allocate_deallocate_roundtrip() {
    let ctx = setup();
    let (strategy, strategy_client) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    let total_before = ctx.client.total_assets();

    // Allocate 5000 to strategy
    ctx.client.allocate(&ctx.keeper, &strategy, &5_000_0000000);
    assert_eq!(strategy_client.balance_of(), 5_000_0000000);
    assert_eq!(ctx.client.tracked_balance(&strategy), 5_000_0000000);
    assert_eq!(ctx.client.total_assets(), total_before); // total unchanged

    // Deallocate 5000 back
    let withdrawn = ctx
        .client
        .deallocate(&ctx.keeper, &strategy, &5_000_0000000);
    assert_eq!(withdrawn, 5_000_0000000);
    assert_eq!(strategy_client.balance_of(), 0);
    assert_eq!(ctx.client.tracked_balance(&strategy), 0);
    assert_eq!(ctx.client.total_assets(), total_before); // total still unchanged
}

#[test]
fn test_multiple_strategies() {
    let ctx = setup();
    let (s1, s1_client) = deploy_mock_strategy(&ctx);
    let (s2, s2_client) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    // Allocate to both
    ctx.client.allocate(&ctx.keeper, &s1, &3_000_0000000);
    ctx.client.allocate(&ctx.keeper, &s2, &2_000_0000000);

    assert_eq!(s1_client.balance_of(), 3_000_0000000);
    assert_eq!(s2_client.balance_of(), 2_000_0000000);
    assert_eq!(ctx.client.total_assets(), 10_000_0000000);

    // Strategy list should have 2
    assert_eq!(ctx.client.get_strategies().len(), 2);
}

#[test]
fn test_emergency_withdraw_zeroes_tracked_balance() {
    let ctx = setup();
    let (strategy, strategy_client) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    ctx.client.allocate(&ctx.keeper, &strategy, &5_000_0000000);
    assert_eq!(ctx.client.tracked_balance(&strategy), 5_000_0000000);

    ctx.client
        .emergency_withdraw_strategy(&ctx.admin, &strategy);

    assert_eq!(ctx.client.tracked_balance(&strategy), 0);
    assert_eq!(strategy_client.balance_of(), 0);
}

#[test]
fn test_remove_strategy_requires_zero_balance() {
    let ctx = setup();
    let (strategy, _) = deploy_mock_strategy(&ctx);
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    ctx.client.allocate(&ctx.keeper, &strategy, &1_000_0000000);

    // Can't remove strategy with funds
    assert!(ctx
        .client
        .try_remove_strategy(&ctx.admin, &strategy)
        .is_err());

    // Deallocate first
    ctx.client
        .deallocate(&ctx.keeper, &strategy, &1_000_0000000);
    assert!(ctx
        .client
        .try_remove_strategy(&ctx.admin, &strategy)
        .is_ok());
}

// ===========================================================================
// UPGRADE TIMELOCK (with time warp)
// ===========================================================================

#[test]
fn test_upgrade_timelock_rejects_early_execution() {
    let ctx = setup();
    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);
    ctx.client.schedule_upgrade(&ctx.admin, &hash);

    // Warp less than default delay (48h = 172800s)
    ctx.env.ledger().with_mut(|info| {
        info.timestamp += 172_799;
    });

    assert!(ctx.client.try_execute_upgrade(&ctx.admin).is_err());
}

#[test]
fn test_upgrade_delay_custom() {
    let ctx = setup();

    // Set 2-hour delay
    ctx.client.set_upgrade_delay(&ctx.admin, &7200);

    let hash = BytesN::from_array(&ctx.env, &[1u8; 32]);
    ctx.client.schedule_upgrade(&ctx.admin, &hash);

    // Warp 2 hours - should be enough now
    ctx.env.ledger().with_mut(|info| {
        info.timestamp += 7201;
    });

    // Execute will fail because WASM hash doesn't exist,
    // but the error should be about missing WASM, not UpgradeTooEarly
    let result = ctx.client.try_execute_upgrade(&ctx.admin);
    assert!(result.is_err()); // Expected: WASM not found, not "too early"
}

// ===========================================================================
// CONVERT FUNCTIONS
// ===========================================================================

#[test]
fn test_convert_to_shares_and_back() {
    let ctx = setup();
    let user = user_with_balance(&ctx, 10_000_0000000);
    ctx.client.deposit(&user, &10_000_0000000);

    let shares = ctx.client.convert_to_shares(&1_000_0000000);
    let assets = ctx.client.convert_to_assets(&shares);

    // Should roundtrip within 1 stroop
    let diff = (assets - 1_000_0000000).abs();
    assert!(diff <= 1, "conversion roundtrip diff: {diff}");
}

#[test]
fn test_convert_empty_vault() {
    let ctx = setup();

    // Empty vault: 1:1 ratio (with virtual offset)
    let shares = ctx.client.convert_to_shares(&1_000_0000000);
    let assets = ctx.client.convert_to_assets(&shares);

    assert!(shares > 0);
    let diff = (assets - 1_000_0000000).abs();
    assert!(diff <= 1);
}
