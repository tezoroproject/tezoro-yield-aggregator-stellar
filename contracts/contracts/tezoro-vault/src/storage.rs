// Storage helpers are pre-built for all DataKey variants.
// Some getters are only used once performance fees / idle buffer are wired.
#![allow(dead_code)]

use soroban_sdk::{contracttype, Address, Env, String, Vec};
use tezoro_common::bump_persistent;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    // -- Instance storage (loaded on every call) --
    Admin,
    PendingAdmin,
    Keeper,
    Guardian,
    Asset,
    Initialized,
    Paused,
    PerformanceFeeBps,
    FeeRecipient,
    IdleBufferBps,
    DepositCap,
    StrategyList,
    VaultName,
    VaultSymbol,
    BridgedMaxAge,
    UpgradeDelay, // seconds before scheduled upgrade can execute

    // -- Persistent storage (independent TTL per entry) --
    TotalSupply,
    HighWaterMark, // nav precision-scaled for fee calculation
    TrackedBalance(Address),
    ShareBalance(Address),
    BridgedBalance,
    BridgedTimestamp,
    ScheduledUpgradeHash,
    ScheduledUpgradeAt, // ledger timestamp when upgrade was scheduled
}

/// Default max age for bridged balance attestation: 24 hours.
pub const DEFAULT_BRIDGED_MAX_AGE: u64 = 86_400;

// ---------------------------------------------------------------------------
// Instance storage helpers
// ---------------------------------------------------------------------------

pub fn get_admin(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Admin).unwrap()
}

pub fn set_admin(e: &Env, admin: &Address) {
    e.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_pending_admin(e: &Env) -> Option<Address> {
    e.storage().instance().get(&DataKey::PendingAdmin)
}

pub fn set_pending_admin(e: &Env, admin: &Address) {
    e.storage().instance().set(&DataKey::PendingAdmin, admin);
}

pub fn clear_pending_admin(e: &Env) {
    e.storage().instance().remove(&DataKey::PendingAdmin);
}

pub fn get_keeper(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Keeper).unwrap()
}

pub fn set_keeper(e: &Env, keeper: &Address) {
    e.storage().instance().set(&DataKey::Keeper, keeper);
}

pub fn get_guardian(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Guardian).unwrap()
}

pub fn set_guardian(e: &Env, guardian: &Address) {
    e.storage().instance().set(&DataKey::Guardian, guardian);
}

pub fn get_asset(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Asset).unwrap()
}

pub fn set_asset(e: &Env, asset: &Address) {
    e.storage().instance().set(&DataKey::Asset, asset);
}

pub fn is_initialized(e: &Env) -> bool {
    e.storage().instance().has(&DataKey::Initialized)
}

pub fn set_initialized(e: &Env) {
    e.storage().instance().set(&DataKey::Initialized, &true);
}

pub fn is_paused(e: &Env) -> bool {
    e.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

pub fn set_paused(e: &Env, paused: bool) {
    e.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn get_performance_fee_bps(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::PerformanceFeeBps)
        .unwrap_or(0)
}

pub fn set_performance_fee_bps(e: &Env, bps: u32) {
    e.storage()
        .instance()
        .set(&DataKey::PerformanceFeeBps, &bps);
}

pub fn get_fee_recipient(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::FeeRecipient).unwrap()
}

pub fn set_fee_recipient(e: &Env, recipient: &Address) {
    e.storage()
        .instance()
        .set(&DataKey::FeeRecipient, recipient);
}

pub fn get_idle_buffer_bps(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::IdleBufferBps)
        .unwrap_or(0)
}

pub fn set_idle_buffer_bps(e: &Env, bps: u32) {
    e.storage().instance().set(&DataKey::IdleBufferBps, &bps);
}

pub fn get_deposit_cap(e: &Env) -> i128 {
    e.storage()
        .instance()
        .get(&DataKey::DepositCap)
        .unwrap_or(0)
}

pub fn set_deposit_cap(e: &Env, cap: i128) {
    e.storage().instance().set(&DataKey::DepositCap, &cap);
}

pub fn get_strategies(e: &Env) -> Vec<Address> {
    e.storage()
        .instance()
        .get(&DataKey::StrategyList)
        .unwrap_or(Vec::new(e))
}

pub fn set_strategies(e: &Env, strategies: &Vec<Address>) {
    e.storage()
        .instance()
        .set(&DataKey::StrategyList, strategies);
}

pub fn get_vault_name(e: &Env) -> String {
    e.storage()
        .instance()
        .get(&DataKey::VaultName)
        .unwrap_or(String::from_str(e, "Tezoro Vault"))
}

pub fn set_vault_name(e: &Env, name: &String) {
    e.storage().instance().set(&DataKey::VaultName, name);
}

pub fn get_vault_symbol(e: &Env) -> String {
    e.storage()
        .instance()
        .get(&DataKey::VaultSymbol)
        .unwrap_or(String::from_str(e, "tVAULT"))
}

pub fn set_vault_symbol(e: &Env, symbol: &String) {
    e.storage().instance().set(&DataKey::VaultSymbol, symbol);
}

pub fn get_bridged_max_age(e: &Env) -> u64 {
    e.storage()
        .instance()
        .get(&DataKey::BridgedMaxAge)
        .unwrap_or(DEFAULT_BRIDGED_MAX_AGE)
}

pub fn set_bridged_max_age(e: &Env, max_age: u64) {
    e.storage()
        .instance()
        .set(&DataKey::BridgedMaxAge, &max_age);
}

// ---------------------------------------------------------------------------
// Persistent storage helpers
// ---------------------------------------------------------------------------

pub fn get_total_supply(e: &Env) -> i128 {
    let key = DataKey::TotalSupply;
    let val: i128 = e.storage().persistent().get(&key).unwrap_or(0);
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_total_supply(e: &Env, supply: i128) {
    let key = DataKey::TotalSupply;
    e.storage().persistent().set(&key, &supply);
    bump_persistent(e, &key);
}

pub fn get_share_balance(e: &Env, account: &Address) -> i128 {
    let key = DataKey::ShareBalance(account.clone());
    let val: i128 = e.storage().persistent().get(&key).unwrap_or(0);
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_share_balance(e: &Env, account: &Address, amount: i128) {
    let key = DataKey::ShareBalance(account.clone());
    e.storage().persistent().set(&key, &amount);
    bump_persistent(e, &key);
}

pub fn get_tracked_balance(e: &Env, strategy: &Address) -> i128 {
    let key = DataKey::TrackedBalance(strategy.clone());
    let val: i128 = e.storage().persistent().get(&key).unwrap_or(0);
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_tracked_balance(e: &Env, strategy: &Address, balance: i128) {
    let key = DataKey::TrackedBalance(strategy.clone());
    e.storage().persistent().set(&key, &balance);
    bump_persistent(e, &key);
}

pub fn remove_tracked_balance(e: &Env, strategy: &Address) {
    let key = DataKey::TrackedBalance(strategy.clone());
    e.storage().persistent().remove(&key);
}

pub fn get_bridged_balance(e: &Env) -> i128 {
    let key = DataKey::BridgedBalance;
    let val: i128 = e.storage().persistent().get(&key).unwrap_or(0);
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_bridged_balance(e: &Env, balance: i128) {
    let key = DataKey::BridgedBalance;
    e.storage().persistent().set(&key, &balance);
    bump_persistent(e, &key);
}

pub fn get_bridged_timestamp(e: &Env) -> u64 {
    let key = DataKey::BridgedTimestamp;
    let val: u64 = e.storage().persistent().get(&key).unwrap_or(0);
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_bridged_timestamp(e: &Env, ts: u64) {
    let key = DataKey::BridgedTimestamp;
    e.storage().persistent().set(&key, &ts);
    bump_persistent(e, &key);
}

// -- Upgrade timelock --

/// Default upgrade delay: 48 hours.
pub const DEFAULT_UPGRADE_DELAY: u64 = 172_800;

pub fn get_upgrade_delay(e: &Env) -> u64 {
    e.storage()
        .instance()
        .get(&DataKey::UpgradeDelay)
        .unwrap_or(DEFAULT_UPGRADE_DELAY)
}

pub fn set_upgrade_delay(e: &Env, delay: u64) {
    e.storage().instance().set(&DataKey::UpgradeDelay, &delay);
}

pub fn get_scheduled_upgrade(e: &Env) -> Option<(soroban_sdk::BytesN<32>, u64)> {
    let hash: Option<soroban_sdk::BytesN<32>> =
        e.storage().persistent().get(&DataKey::ScheduledUpgradeHash);
    let at: Option<u64> = e.storage().persistent().get(&DataKey::ScheduledUpgradeAt);
    match (hash, at) {
        (Some(h), Some(t)) => Some((h, t)),
        _ => None,
    }
}

pub fn set_scheduled_upgrade(e: &Env, hash: &soroban_sdk::BytesN<32>, at: u64) {
    e.storage()
        .persistent()
        .set(&DataKey::ScheduledUpgradeHash, hash);
    e.storage()
        .persistent()
        .set(&DataKey::ScheduledUpgradeAt, &at);
    bump_persistent(e, &DataKey::ScheduledUpgradeHash);
    bump_persistent(e, &DataKey::ScheduledUpgradeAt);
}

pub fn clear_scheduled_upgrade(e: &Env) {
    e.storage()
        .persistent()
        .remove(&DataKey::ScheduledUpgradeHash);
    e.storage()
        .persistent()
        .remove(&DataKey::ScheduledUpgradeAt);
}

// -- High Water Mark (for performance fees) --

/// Precision multiplier for NAV-per-share calculation (7 decimals).
pub const NAV_PRECISION: i128 = 10_000_000;

pub fn get_high_water_mark(e: &Env) -> i128 {
    let key = DataKey::HighWaterMark;
    let val: i128 = e.storage().persistent().get(&key).unwrap_or(NAV_PRECISION); // default 1:1
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_high_water_mark(e: &Env, hwm: i128) {
    let key = DataKey::HighWaterMark;
    e.storage().persistent().set(&key, &hwm);
    bump_persistent(e, &key);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a strategy address exists in the strategy list.
pub fn strategy_exists(e: &Env, strategy: &Address) -> bool {
    let strategies = get_strategies(e);
    for s in strategies.iter() {
        if s == *strategy {
            return true;
        }
    }
    false
}
