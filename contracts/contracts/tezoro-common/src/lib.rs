// SPDX-License-Identifier: UNLICENSED
#![no_std]

mod strategy;

pub use strategy::StrategyClient;

use soroban_sdk::{contracterror, Env};

// ---------------------------------------------------------------------------
// Strategy Errors (shared by all strategy contracts)
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StrategyError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    InsufficientBalance = 4,
    ZeroAmount = 5,
    Unhealthy = 6,
    InvalidBps = 7,
    Paused = 8,
    UpgradeNotScheduled = 9,
    UpgradeTooEarly = 10,
}

// ---------------------------------------------------------------------------
// TTL Constants
// ---------------------------------------------------------------------------

/// Approximate ledgers per day at 5s/ledger.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// Instance storage: extend when TTL drops below ~30 days.
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;
/// Instance storage: extend to ~120 days.
pub const INSTANCE_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 120;

/// Persistent storage: extend when TTL drops below ~30 days.
pub const PERSISTENT_LIFETIME_THRESHOLD: u32 = LEDGERS_PER_DAY * 30;
/// Persistent storage: extend to ~120 days.
pub const PERSISTENT_BUMP_AMOUNT: u32 = LEDGERS_PER_DAY * 120;

// ---------------------------------------------------------------------------
// BPS Utilities
// ---------------------------------------------------------------------------

pub const MAX_BPS: u32 = 10_000;

/// Extend instance TTL using standard thresholds.
pub fn bump_instance(e: &Env) {
    e.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Extend a persistent entry's TTL using standard thresholds.
pub fn bump_persistent<K>(e: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    e.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
}
