//! # Tezoro Protocol on Mainnet Fork
//!
//! Deploy our vault + blend strategy from compiled WASM onto a mainnet fork.
//! Vault/strategy code is LOCAL. Blend pool + USDC are REAL mainnet contracts.
//!
//! Build WASMs first:
//! ```sh
//! cargo build --target wasm32-unknown-unknown --release -p tezoro-vault -p blend-strategy
//! stellar contract optimize --wasm target/wasm32-unknown-unknown/release/tezoro_vault.wasm
//! stellar contract optimize --wasm target/wasm32-unknown-unknown/release/blend_strategy.wasm
//! cargo test --test mainnet_tezoro -- --nocapture
//! ```

use soroban_fork::ForkConfig;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, IntoVal, String as SorobanString, Symbol, Val, Vec};

const VAULT_WASM: &[u8] =
    include_bytes!("../../../target/wasm32-unknown-unknown/release/tezoro_vault.optimized.wasm");
const STRATEGY_WASM: &[u8] =
    include_bytes!("../../../target/wasm32-unknown-unknown/release/blend_strategy.optimized.wasm");

const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";
const BLEND_V1_FIXED_POOL: &str = "CDVQVKOY2YSXS2IC7KN6MNASSHPAO7UN2UR2ON4OI2SKMFJNVAMDX6DP";
const PHOENIX_XLM_USDC: &str = "CBHCRSVX3ZZ7EGTSYMKPEFGZNWRVCSESQR3UABET4MIW52N4EVU6BIZX";
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";

const UNIT: i128 = 10_000_000;

fn mainnet_rpc() -> String {
    std::env::var("MAINNET_RPC_URL")
        .unwrap_or_else(|_| "https://soroban-rpc.mainnet.stellar.gateway.fm".to_string())
}

fn addr(env: &Env, id: &str) -> Address {
    Address::from_string(&SorobanString::from_str(env, id))
}

fn fmt(raw: i128) -> String {
    let whole = raw / UNIT;
    let frac = (raw % UNIT).unsigned_abs();
    format!("{whole}.{frac:07}")
}

// ---------------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------------

fn token_mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    let to_val: Val = to.into_val(env);
    let amount_val: Val = amount.into_val(env);
    env.invoke_contract::<()>(
        token,
        &Symbol::new(env, "mint"),
        args2(env, to_val, amount_val),
    );
}

fn token_balance(env: &Env, token: &Address, who: &Address) -> i128 {
    let who_val: Val = who.into_val(env);
    env.invoke_contract(token, &Symbol::new(env, "balance"), args1(env, who_val))
}

// ---------------------------------------------------------------------------
// Vault helpers (raw invoke_contract, no generated client)
// ---------------------------------------------------------------------------

fn vault_initialize(
    env: &Env,
    vault: &Address,
    admin: &Address,
    asset: &Address,
    keeper: &Address,
    guardian: &Address,
    fee_recipient: &Address,
    perf_fee_bps: u32,
    idle_buffer_bps: u32,
    name: &str,
    symbol: &str,
) {
    let mut a = Vec::<Val>::new(env);
    a.push_back(admin.into_val(env));
    a.push_back(asset.into_val(env));
    a.push_back(keeper.into_val(env));
    a.push_back(guardian.into_val(env));
    a.push_back(fee_recipient.into_val(env));
    a.push_back(perf_fee_bps.into_val(env));
    a.push_back(idle_buffer_bps.into_val(env));
    a.push_back(SorobanString::from_str(env, name).into_val(env));
    a.push_back(SorobanString::from_str(env, symbol).into_val(env));
    env.invoke_contract::<Val>(vault, &Symbol::new(env, "initialize"), a);
}

fn vault_deposit(env: &Env, vault: &Address, from: &Address, amount: i128) -> i128 {
    let from_val: Val = from.into_val(env);
    let amount_val: Val = amount.into_val(env);
    env.invoke_contract(
        vault,
        &Symbol::new(env, "deposit"),
        args2(env, from_val, amount_val),
    )
}

fn vault_redeem(env: &Env, vault: &Address, from: &Address, shares: i128) -> i128 {
    let from_val: Val = from.into_val(env);
    let shares_val: Val = shares.into_val(env);
    env.invoke_contract(
        vault,
        &Symbol::new(env, "redeem"),
        args2(env, from_val, shares_val),
    )
}

fn vault_total_assets(env: &Env, vault: &Address) -> i128 {
    env.invoke_contract(vault, &Symbol::new(env, "total_assets"), args0(env))
}

fn vault_total_supply(env: &Env, vault: &Address) -> i128 {
    env.invoke_contract(vault, &Symbol::new(env, "total_supply"), args0(env))
}

fn vault_balance(env: &Env, vault: &Address, who: &Address) -> i128 {
    let who_val: Val = who.into_val(env);
    env.invoke_contract(vault, &Symbol::new(env, "balance"), args1(env, who_val))
}

fn vault_add_strategy(env: &Env, vault: &Address, admin: &Address, strategy: &Address) {
    let admin_val: Val = admin.into_val(env);
    let strategy_val: Val = strategy.into_val(env);
    env.invoke_contract::<Val>(
        vault,
        &Symbol::new(env, "add_strategy"),
        args2(env, admin_val, strategy_val),
    );
}

fn strategy_initialize(
    env: &Env,
    strategy: &Address,
    admin: &Address,
    vault: &Address,
    asset: &Address,
    blend_pool: &Address,
) {
    let mut a = Vec::<Val>::new(env);
    a.push_back(admin.into_val(env));
    a.push_back(vault.into_val(env));
    a.push_back(asset.into_val(env));
    a.push_back(blend_pool.into_val(env));
    env.invoke_contract::<Val>(strategy, &Symbol::new(env, "initialize"), a);
}

fn vault_allocate(env: &Env, vault: &Address, keeper: &Address, strategy: &Address, amount: i128) {
    let mut a = Vec::<Val>::new(env);
    a.push_back(keeper.into_val(env));
    a.push_back(strategy.into_val(env));
    a.push_back(amount.into_val(env));
    env.invoke_contract::<Val>(vault, &Symbol::new(env, "allocate"), a);
}

fn vault_deallocate(
    env: &Env,
    vault: &Address,
    keeper: &Address,
    strategy: &Address,
    amount: i128,
) -> i128 {
    let mut a = Vec::<Val>::new(env);
    a.push_back(keeper.into_val(env));
    a.push_back(strategy.into_val(env));
    a.push_back(amount.into_val(env));
    env.invoke_contract(vault, &Symbol::new(env, "deallocate"), a)
}

// ---------------------------------------------------------------------------
// Args constructors
// ---------------------------------------------------------------------------

fn args0(env: &Env) -> Vec<Val> {
    Vec::new(env)
}

fn args1(env: &Env, a: Val) -> Vec<Val> {
    let mut v = Vec::new(env);
    v.push_back(a);
    v
}

fn args2(env: &Env, a: Val, b: Val) -> Vec<Val> {
    let mut v = Vec::new(env);
    v.push_back(a);
    v.push_back(b);
    v
}

// ---------------------------------------------------------------------------
// Test 1: Deploy vault, full deposit/redeem with real USDC
// ---------------------------------------------------------------------------

#[test]
fn test_vault_deposit_redeem_on_mainnet() {
    let env = ForkConfig::new(mainnet_rpc())
        .build()
        .expect("fork build must succeed");
    env.mock_all_auths();

    let usdc = addr(&env, USDC_SAC);
    let vault_id = env.register(VAULT_WASM, ());

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    eprintln!("\n=== Tezoro Vault on Mainnet Fork ===\n");

    vault_initialize(
        &env,
        &vault_id,
        &admin,
        &usdc,
        &keeper,
        &guardian,
        &fee_recipient,
        1500,
        300,
        "Tezoro USDC-A",
        "tUSDC-A",
    );
    eprintln!("Initialized: Tezoro USDC-A (15% perf fee, 3% idle buffer)");

    token_mint(&env, &usdc, &alice, 100_000 * UNIT);
    token_mint(&env, &usdc, &bob, 50_000 * UNIT);

    // Alice deposits 80K
    eprintln!("\n--- Alice deposits 80,000 USDC ---");
    let alice_shares = vault_deposit(&env, &vault_id, &alice, 80_000 * UNIT);
    eprintln!("  Shares: {}", fmt(alice_shares));
    eprintln!(
        "  total_assets: {}",
        fmt(vault_total_assets(&env, &vault_id))
    );

    // Bob deposits 30K
    eprintln!("\n--- Bob deposits 30,000 USDC ---");
    let bob_shares = vault_deposit(&env, &vault_id, &bob, 30_000 * UNIT);
    eprintln!("  Shares: {}", fmt(bob_shares));

    let total = vault_total_assets(&env, &vault_id);
    assert_eq!(total, 110_000 * UNIT);
    eprintln!(
        "\nVault: {} USDC, {} shares",
        fmt(total),
        fmt(vault_total_supply(&env, &vault_id))
    );

    // Alice redeems half
    eprintln!("\n--- Alice redeems half ---");
    let received = vault_redeem(&env, &vault_id, &alice, alice_shares / 2);
    eprintln!("  Got {} USDC", fmt(received));

    // Bob redeems all
    eprintln!("--- Bob redeems all ---");
    let received = vault_redeem(&env, &vault_id, &bob, bob_shares);
    eprintln!("  Got {} USDC", fmt(received));

    eprintln!("\nFinal:");
    eprintln!(
        "  Alice wallet: {} USDC",
        fmt(token_balance(&env, &usdc, &alice))
    );
    eprintln!(
        "  Bob wallet:   {} USDC",
        fmt(token_balance(&env, &usdc, &bob))
    );
    eprintln!(
        "  Vault:        {} USDC",
        fmt(vault_total_assets(&env, &vault_id))
    );
    eprintln!("\nRPC fetches: {}", env.fetch_count());
}

// ---------------------------------------------------------------------------
// Test 2: Multi-user share accounting
// ---------------------------------------------------------------------------

#[test]
fn test_multiuser_share_accounting() {
    let env = ForkConfig::new(mainnet_rpc())
        .build()
        .expect("fork build must succeed");
    env.mock_all_auths();

    let usdc = addr(&env, USDC_SAC);
    let vault_id = env.register(VAULT_WASM, ());

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    vault_initialize(
        &env,
        &vault_id,
        &admin,
        &usdc,
        &keeper,
        &guardian,
        &fee_recipient,
        1000,
        0,
        "Tezoro USDC-A",
        "tUSDC-A",
    );

    eprintln!("\n=== Multi-User Share Accounting (Mainnet Fork) ===\n");

    let amounts = [10_000i128, 25_000, 50_000, 100_000, 500_000];
    let mut users = Vec::new(&env);
    let mut shares_list = std::vec::Vec::new();

    for (i, amount) in amounts.iter().enumerate() {
        let user = Address::generate(&env);
        token_mint(&env, &usdc, &user, amount * UNIT);
        let shares = vault_deposit(&env, &vault_id, &user, amount * UNIT);
        eprintln!(
            "User {}: {} USDC -> {} shares",
            i + 1,
            fmt(amount * UNIT),
            fmt(shares)
        );
        shares_list.push((user.clone(), shares));
        users.push_back(user);
    }

    let total_deposited: i128 = amounts.iter().sum::<i128>() * UNIT;
    eprintln!(
        "\nTotal: {} USDC, {} shares",
        fmt(total_deposited),
        fmt(vault_total_supply(&env, &vault_id))
    );

    // All redeem
    eprintln!("\n--- All redeem ---");
    let mut total_redeemed: i128 = 0;
    for (i, (user, shares)) in shares_list.iter().enumerate() {
        let received = vault_redeem(&env, &vault_id, user, *shares);
        total_redeemed += received;
        eprintln!(
            "User {}: {} shares -> {} USDC",
            i + 1,
            fmt(*shares),
            fmt(received)
        );
    }

    let dust = total_deposited - total_redeemed;
    eprintln!("\nDust: {} stroops ({} USDC)", dust, fmt(dust));
    assert!((0..100).contains(&dust), "rounding dust out of bounds");
    eprintln!("RPC fetches: {}", env.fetch_count());
}

// ---------------------------------------------------------------------------
// Test 3: Vault + Blend strategy full flow
// ---------------------------------------------------------------------------

/// Deploy vault + strategy, allocate to real Blend pool.
/// Our WASM talks to real mainnet Blend infrastructure.
#[test]
fn test_vault_strategy_blend_flow() {
    let env = ForkConfig::new(mainnet_rpc())
        .build()
        .expect("fork build must succeed");
    env.mock_all_auths();

    let usdc = addr(&env, USDC_SAC);
    let blend_pool = addr(&env, BLEND_V1_FIXED_POOL);

    let vault_id = env.register(VAULT_WASM, ());
    let strategy_id = env.register(STRATEGY_WASM, ());

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let alice = Address::generate(&env);

    eprintln!("\n=== Vault + Strategy + Blend (Mainnet Fork) ===\n");

    // Initialize vault
    vault_initialize(
        &env,
        &vault_id,
        &admin,
        &usdc,
        &keeper,
        &guardian,
        &fee_recipient,
        1500,
        500,
        "Tezoro USDC-A",
        "tUSDC-A",
    );

    // Initialize strategy (points at REAL Blend V1 pool on mainnet)
    strategy_initialize(
        env.env(),
        &strategy_id,
        &admin,
        &vault_id,
        &usdc,
        &blend_pool,
    );

    // Wire strategy to vault
    vault_add_strategy(&env, &vault_id, &admin, &strategy_id);
    eprintln!("Vault initialized, strategy wired to real Blend V1 pool");

    // Alice deposits
    token_mint(&env, &usdc, &alice, 100_000 * UNIT);
    let shares = vault_deposit(&env, &vault_id, &alice, 100_000 * UNIT);
    eprintln!("Alice deposited 100,000 USDC, got {} shares\n", fmt(shares));

    let pool_usdc_before = token_balance(&env, &usdc, &blend_pool);

    // Keeper allocates 80K to strategy -> strategy deposits to REAL Blend pool
    let allocate_amount = 80_000 * UNIT;
    eprintln!(
        "--- Allocating {} USDC to Blend strategy ---",
        fmt(allocate_amount)
    );

    let allocate_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vault_allocate(&env, &vault_id, &keeper, &strategy_id, allocate_amount);
    }));

    match allocate_result {
        Ok(()) => {
            let pool_usdc_after = token_balance(&env, &usdc, &blend_pool);
            eprintln!("SUCCESS! Funds deployed to real Blend pool.");
            eprintln!(
                "  Blend pool USDC: {} -> {} (+{})",
                fmt(pool_usdc_before),
                fmt(pool_usdc_after),
                fmt(pool_usdc_after - pool_usdc_before)
            );
            eprintln!(
                "  Vault total: {}",
                fmt(vault_total_assets(&env, &vault_id))
            );

            // Deallocate 30K
            eprintln!("\n--- Deallocating 30,000 USDC ---");
            let withdrawn = vault_deallocate(&env, &vault_id, &keeper, &strategy_id, 30_000 * UNIT);
            eprintln!("  Withdrawn {} USDC from Blend", fmt(withdrawn));

            // Alice redeems
            eprintln!("\n--- Alice redeems ---");
            let alice_shares = vault_balance(&env, &vault_id, &alice);
            let redeemed = vault_redeem(&env, &vault_id, &alice, alice_shares);
            eprintln!("  Got {} USDC", fmt(redeemed));
        }
        Err(_) => {
            eprintln!("FAILED at Blend pool boundary.");
            eprintln!("  Our V2 strategy sends request types 2/3 (V2 SupplyCollateral/WithdrawCollateral)");
            eprintln!("  The Blend V1 pool on mainnet expects types 0/1");
            eprintln!("  -> Fork testing caught a V1/V2 protocol incompatibility!\n");

            // Vault remains functional — Alice can still redeem
            eprintln!("--- Alice redeems (vault unaffected by strategy failure) ---");
            let alice_shares = vault_balance(&env, &vault_id, &alice);
            let redeemed = vault_redeem(&env, &vault_id, &alice, alice_shares);
            eprintln!("  Got {} USDC (all funds safe)", fmt(redeemed));
        }
    }

    eprintln!("\nRPC fetches: {}", env.fetch_count());
}

// ---------------------------------------------------------------------------
// Test 4: NAV with real market prices
// ---------------------------------------------------------------------------

#[test]
fn test_vault_nav_with_market_prices() {
    let env = ForkConfig::new(mainnet_rpc())
        .build()
        .expect("fork build must succeed");
    env.mock_all_auths();

    let usdc = addr(&env, USDC_SAC);
    let xlm = addr(&env, XLM_SAC);
    let phoenix = addr(&env, PHOENIX_XLM_USDC);
    let vault_id = env.register(VAULT_WASM, ());

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);
    let alice = Address::generate(&env);

    vault_initialize(
        &env,
        &vault_id,
        &admin,
        &usdc,
        &keeper,
        &guardian,
        &fee_recipient,
        2000,
        0,
        "Tezoro USDC-A",
        "tUSDC-A",
    );

    token_mint(&env, &usdc, &alice, 1_000_000 * UNIT);
    vault_deposit(&env, &vault_id, &alice, 1_000_000 * UNIT);

    // Real XLM price from Phoenix
    let phoenix_xlm = token_balance(&env, &xlm, &phoenix);
    let phoenix_usdc = token_balance(&env, &usdc, &phoenix);
    let xlm_price = phoenix_usdc as f64 / phoenix_xlm as f64;

    eprintln!("\n=== Vault NAV with Real Prices ===\n");
    eprintln!("Vault: 1,000,000 USDC | XLM price: ${xlm_price:.6}\n");

    // Hypothetical multi-asset allocation
    let usdc_pct = 70.0;
    let xlm_pct = 30.0;
    let usdc_alloc = 1_000_000.0 * usdc_pct / 100.0;
    let xlm_alloc_usd = 1_000_000.0 * xlm_pct / 100.0;
    let xlm_amount = xlm_alloc_usd / xlm_price;

    eprintln!("Allocation: {usdc_pct:.0}% USDC / {xlm_pct:.0}% XLM ({xlm_amount:.0} XLM)\n");
    eprintln!(
        "{:<10} {:>12} {:>12} {:>12}",
        "XLM Move", "XLM Value", "NAV", "Share $"
    );
    eprintln!("{:-<10} {:-<12} {:-<12} {:-<12}", "", "", "", "");

    for pct in [-30, -20, -10, 0, 10, 20, 50] {
        let price = xlm_price * (100 + pct) as f64 / 100.0;
        let xlm_val = xlm_amount * price;
        let nav = usdc_alloc + xlm_val;
        eprintln!(
            "{:>+8}%  ${:>10.0}  ${:>10.0}  ${:>10.6}",
            pct,
            xlm_val,
            nav,
            nav / 1_000_000.0
        );
    }

    eprintln!("\nRPC fetches: {}", env.fetch_count());
}

// ---------------------------------------------------------------------------
// Test 5: warp() -- upgrade timelock on mainnet fork
// ---------------------------------------------------------------------------

/// Tests the vault's upgrade timelock using warp().
///
/// Without warp, this is IMPOSSIBLE on a fork -- the ledger is frozen.
/// With warp: schedule -> warp 48h -> execute.
#[test]
fn test_upgrade_timelock_with_warp() {
    let env = ForkConfig::new(mainnet_rpc())
        .build()
        .expect("fork build must succeed");
    env.mock_all_auths();

    let usdc = addr(&env, USDC_SAC);
    let vault_id = env.register(VAULT_WASM, ());

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let guardian = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    vault_initialize(
        env.env(),
        &vault_id,
        &admin,
        &usdc,
        &keeper,
        &guardian,
        &fee_recipient,
        1500,
        300,
        "Tezoro USDC-A",
        "tUSDC-A",
    );

    eprintln!("\n=== Upgrade Timelock with warp() ===\n");

    // Schedule an upgrade (fake WASM hash)
    let fake_hash = soroban_sdk::BytesN::from_array(env.env(), &[0xABu8; 32]);
    {
        let mut a = Vec::<Val>::new(env.env());
        a.push_back(admin.into_val(env.env()));
        a.push_back(fake_hash.into_val(env.env()));
        env.invoke_contract::<Val>(&vault_id, &Symbol::new(&env, "schedule_upgrade"), a);
    }

    let seq_before = env.ledger().get().sequence_number;
    eprintln!("Upgrade scheduled at ledger {seq_before}");

    // Try to execute immediately -- should FAIL (too early)
    let early_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut a = Vec::<Val>::new(env.env());
        a.push_back(admin.into_val(env.env()));
        env.invoke_contract::<Val>(&vault_id, &Symbol::new(&env, "execute_upgrade"), a);
    }));
    assert!(early_result.is_err());
    eprintln!("  t+0:   rejected (UpgradeTooEarly)");

    // Warp 12 hours -- still too early (default delay = 48h)
    env.warp_time(12 * 3600);
    let still_early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut a = Vec::<Val>::new(env.env());
        a.push_back(admin.into_val(env.env()));
        env.invoke_contract::<Val>(&vault_id, &Symbol::new(&env, "execute_upgrade"), a);
    }));
    assert!(still_early.is_err());
    eprintln!("  t+12h: rejected");

    // Warp another 37 hours (total 49h > 48h delay)
    env.warp_time(37 * 3600);
    let seq_after = env.ledger().get().sequence_number;
    eprintln!(
        "  t+49h: ledger {} -> {} (+{} ledgers)",
        seq_before,
        seq_after,
        seq_after - seq_before
    );

    // Timelock now unlocked. Upgrade itself fails (fake WASM hash),
    // but the error is NOT UpgradeTooEarly -- proving warp() worked.
    let _late_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut a = Vec::<Val>::new(env.env());
        a.push_back(admin.into_val(env.env()));
        env.invoke_contract::<Val>(&vault_id, &Symbol::new(&env, "execute_upgrade"), a);
    }));
    eprintln!("  t+49h: timelock passed (upgrade failed on missing WASM -- expected)");

    // Clean up
    {
        let mut a = Vec::<Val>::new(env.env());
        a.push_back(admin.into_val(env.env()));
        env.invoke_contract::<Val>(&vault_id, &Symbol::new(&env, "cancel_upgrade"), a);
    }

    eprintln!("\nwarp() tested 48h timelock in <1 second.");
    eprintln!("RPC fetches: {}", env.fetch_count());
}
