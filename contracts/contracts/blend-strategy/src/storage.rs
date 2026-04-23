#![allow(dead_code)] // Storage helpers pre-built for all keys

use soroban_sdk::{contracttype, Address, Env};
use tezoro_common::bump_persistent;

pub const DEFAULT_MAX_UTILIZATION_BPS: u32 = 9_500; // 95%
pub const DEFAULT_MIN_BACKSTOP_BPS: u32 = 500; // 5% minimum coverage
pub const DEFAULT_APPROVAL_BUFFER: u32 = 200; // ~17 min at 5s/ledger

pub const DEFAULT_UPGRADE_DELAY: u64 = 172_800; // 48 hours
pub const MIN_UPGRADE_DELAY: u64 = 3_600; // 1 hour

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Initialized,
    Admin,
    PendingAdmin,
    Vault,
    Asset,
    BlendPool,
    Paused,
    TrackedBalance,
    MaxUtilizationBps,
    MinBackstopCoverageBps,
    ApprovalLedgerBuffer,
    UpgradeDelay,
    ScheduledUpgradeHash,
    ScheduledUpgradeAt,
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

// -- Instance helpers --

pub fn is_initialized(e: &Env) -> bool {
    e.storage().instance().has(&DataKey::Initialized)
}

pub fn set_initialized(e: &Env) {
    e.storage().instance().set(&DataKey::Initialized, &true);
}

pub fn get_admin(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Admin).unwrap()
}

pub fn set_admin(e: &Env, admin: &Address) {
    e.storage().instance().set(&DataKey::Admin, admin);
}

pub fn get_vault(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Vault).unwrap()
}

pub fn set_vault(e: &Env, vault: &Address) {
    e.storage().instance().set(&DataKey::Vault, vault);
}

pub fn get_asset(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::Asset).unwrap()
}

pub fn set_asset(e: &Env, asset: &Address) {
    e.storage().instance().set(&DataKey::Asset, asset);
}

pub fn get_blend_pool(e: &Env) -> Address {
    e.storage().instance().get(&DataKey::BlendPool).unwrap()
}

pub fn set_blend_pool(e: &Env, pool: &Address) {
    e.storage().instance().set(&DataKey::BlendPool, pool);
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

pub fn get_max_utilization_bps(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::MaxUtilizationBps)
        .unwrap_or(DEFAULT_MAX_UTILIZATION_BPS)
}

pub fn set_max_utilization_bps(e: &Env, bps: u32) {
    e.storage()
        .instance()
        .set(&DataKey::MaxUtilizationBps, &bps);
}

pub fn get_min_backstop_bps(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::MinBackstopCoverageBps)
        .unwrap_or(DEFAULT_MIN_BACKSTOP_BPS)
}

pub fn set_min_backstop_bps(e: &Env, bps: u32) {
    e.storage()
        .instance()
        .set(&DataKey::MinBackstopCoverageBps, &bps);
}

pub fn get_approval_buffer(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::ApprovalLedgerBuffer)
        .unwrap_or(DEFAULT_APPROVAL_BUFFER)
}

pub fn set_approval_buffer(e: &Env, buffer: u32) {
    e.storage()
        .instance()
        .set(&DataKey::ApprovalLedgerBuffer, &buffer);
}

// -- Persistent helpers --

pub fn get_tracked_balance(e: &Env) -> i128 {
    let key = DataKey::TrackedBalance;
    let val: i128 = e.storage().persistent().get(&key).unwrap_or(0);
    if val != 0 {
        bump_persistent(e, &key);
    }
    val
}

pub fn set_tracked_balance(e: &Env, balance: i128) {
    let key = DataKey::TrackedBalance;
    e.storage().persistent().set(&key, &balance);
    bump_persistent(e, &key);
}

// -- Upgrade timelock --

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
