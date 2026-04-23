use soroban_sdk::{contractevent, Address, BytesN, Env, Symbol};

#[contractevent]
pub struct Deposit {
    #[topic]
    pub from: Address,
    pub assets: i128,
    pub shares: i128,
}

#[contractevent]
pub struct Redeem {
    #[topic]
    pub from: Address,
    pub shares: i128,
    pub assets: i128,
}

/// SEP-41 standard transfer event.
#[contractevent]
pub struct Transfer {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub amount: i128,
}

#[contractevent]
pub struct StrategyAdd {
    pub strategy: Address,
}

#[contractevent]
pub struct StrategyRemove {
    pub strategy: Address,
}

#[contractevent]
pub struct Pause {
    pub caller: Address,
}

#[contractevent]
pub struct Unpause {
    pub caller: Address,
}

#[contractevent]
pub struct BridgedUpdate {
    pub balance: i128,
    pub timestamp: u64,
}

#[contractevent]
pub struct TrackedUpdate {
    #[topic]
    pub strategy: Address,
    pub balance: i128,
}

#[contractevent]
pub struct Upgrade {
    pub new_wasm_hash: BytesN<32>,
}

#[contractevent]
pub struct SetRole {
    #[topic]
    pub role: Symbol,
    pub new_address: Address,
}

#[contractevent]
pub struct DepositCapUpdate {
    pub cap: i128,
}

#[contractevent]
pub struct AdminProposed {
    pub current: Address,
    pub proposed: Address,
}

#[contractevent]
pub struct AdminAccepted {
    pub new_admin: Address,
}

#[contractevent]
pub struct Allocate {
    #[topic]
    pub strategy: Address,
    pub amount: i128,
}

#[contractevent]
pub struct Deallocate {
    #[topic]
    pub strategy: Address,
    pub amount: i128,
}

#[contractevent]
pub struct UpgradeScheduled {
    pub wasm_hash: BytesN<32>,
    pub execute_after: u64,
}

#[contractevent]
pub struct UpgradeCancelled {}

#[contractevent]
pub struct FeesCollected {
    pub fee_shares: i128,
    pub new_hwm: i128,
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

pub fn deposit(e: &Env, from: &Address, assets: i128, shares: i128) {
    Deposit {
        from: from.clone(),
        assets,
        shares,
    }
    .publish(e);
}

pub fn redeem(e: &Env, from: &Address, shares: i128, assets: i128) {
    Redeem {
        from: from.clone(),
        shares,
        assets,
    }
    .publish(e);
}

pub fn transfer(e: &Env, from: &Address, to: &Address, amount: i128) {
    Transfer {
        from: from.clone(),
        to: to.clone(),
        amount,
    }
    .publish(e);
}

pub fn strategy_add(e: &Env, strategy: &Address) {
    StrategyAdd {
        strategy: strategy.clone(),
    }
    .publish(e);
}

pub fn strategy_remove(e: &Env, strategy: &Address) {
    StrategyRemove {
        strategy: strategy.clone(),
    }
    .publish(e);
}

pub fn pause(e: &Env, caller: &Address) {
    Pause {
        caller: caller.clone(),
    }
    .publish(e);
}

pub fn unpause(e: &Env, caller: &Address) {
    Unpause {
        caller: caller.clone(),
    }
    .publish(e);
}

pub fn bridged_update(e: &Env, balance: i128, timestamp: u64) {
    BridgedUpdate { balance, timestamp }.publish(e);
}

pub fn tracked_update(e: &Env, strategy: &Address, balance: i128) {
    TrackedUpdate {
        strategy: strategy.clone(),
        balance,
    }
    .publish(e);
}

pub fn upgrade(e: &Env, new_wasm_hash: &BytesN<32>) {
    Upgrade {
        new_wasm_hash: new_wasm_hash.clone(),
    }
    .publish(e);
}

pub fn set_role(e: &Env, role: &str, new_address: &Address) {
    SetRole {
        role: Symbol::new(e, role),
        new_address: new_address.clone(),
    }
    .publish(e);
}

pub fn deposit_cap(e: &Env, cap: i128) {
    DepositCapUpdate { cap }.publish(e);
}

pub fn admin_proposed(e: &Env, current: &Address, proposed: &Address) {
    AdminProposed {
        current: current.clone(),
        proposed: proposed.clone(),
    }
    .publish(e);
}

pub fn admin_accepted(e: &Env, new_admin: &Address) {
    AdminAccepted {
        new_admin: new_admin.clone(),
    }
    .publish(e);
}

pub fn allocate(e: &Env, strategy: &Address, amount: i128) {
    Allocate {
        strategy: strategy.clone(),
        amount,
    }
    .publish(e);
}

pub fn deallocate(e: &Env, strategy: &Address, amount: i128) {
    Deallocate {
        strategy: strategy.clone(),
        amount,
    }
    .publish(e);
}

pub fn upgrade_scheduled(e: &Env, wasm_hash: &BytesN<32>, execute_after: u64) {
    UpgradeScheduled {
        wasm_hash: wasm_hash.clone(),
        execute_after,
    }
    .publish(e);
}

pub fn upgrade_cancelled(e: &Env) {
    UpgradeCancelled {}.publish(e);
}

pub fn fees_collected(e: &Env, fee_shares: i128, new_hwm: i128) {
    FeesCollected {
        fee_shares,
        new_hwm,
    }
    .publish(e);
}
