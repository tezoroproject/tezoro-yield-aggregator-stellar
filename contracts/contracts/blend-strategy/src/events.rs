use soroban_sdk::{contractevent, Address, BytesN, Env};

#[contractevent]
pub struct Deposit {
    pub amount: i128,
}

#[contractevent]
pub struct Withdraw {
    pub amount: i128,
    pub requested: i128,
}

#[contractevent]
pub struct EmergencyWithdraw {
    pub amount: i128,
    pub caller: Address,
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
pub struct ConfigUpdate {
    pub key: soroban_sdk::Symbol,
    pub value: u32,
}

#[contractevent]
pub struct AdminChanged {
    pub old_admin: Address,
    pub new_admin: Address,
}

#[contractevent]
pub struct VaultChanged {
    pub old_vault: Address,
    pub new_vault: Address,
}

#[contractevent]
pub struct Upgrade {
    pub new_wasm_hash: BytesN<32>,
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

pub fn deposit(e: &Env, amount: i128) {
    Deposit { amount }.publish(e);
}

pub fn withdraw(e: &Env, amount: i128, requested: i128) {
    Withdraw { amount, requested }.publish(e);
}

pub fn emergency_withdraw(e: &Env, amount: i128, caller: &Address) {
    EmergencyWithdraw {
        amount,
        caller: caller.clone(),
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

pub fn config_update(e: &Env, key: &str, value: u32) {
    ConfigUpdate {
        key: soroban_sdk::Symbol::new(e, key),
        value,
    }
    .publish(e);
}

pub fn admin_changed(e: &Env, old_admin: &Address, new_admin: &Address) {
    AdminChanged {
        old_admin: old_admin.clone(),
        new_admin: new_admin.clone(),
    }
    .publish(e);
}

pub fn vault_changed(e: &Env, old_vault: &Address, new_vault: &Address) {
    VaultChanged {
        old_vault: old_vault.clone(),
        new_vault: new_vault.clone(),
    }
    .publish(e);
}

pub fn upgrade(e: &Env, new_wasm_hash: &BytesN<32>) {
    Upgrade {
        new_wasm_hash: new_wasm_hash.clone(),
    }
    .publish(e);
}
