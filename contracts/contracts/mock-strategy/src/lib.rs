// SPDX-License-Identifier: UNLICENSED
#![no_std]

use soroban_sdk::{contract, contractevent, contractimpl, contracttype, token, Address, Env};
use tezoro_common::{bump_instance, bump_persistent, StrategyError};

#[contractevent]
pub struct DepositEvent {
    pub amount: i128,
}

#[contractevent]
pub struct WithdrawEvent {
    pub amount: i128,
}

#[contractevent]
pub struct EmergencyWithdrawEvent {
    pub amount: i128,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Initialized,
    Admin,
    Vault,
    Asset,
    Balance,
    Healthy,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Mock strategy for testing vault integration.
///
/// Simulates a lending protocol by holding tokens and allowing
/// the admin to set a fake yield (increase tracked balance).
#[contract]
pub struct MockStrategy;

#[contractimpl]
impl MockStrategy {
    pub fn initialize(
        e: Env,
        admin: Address,
        vault: Address,
        asset: Address,
    ) -> Result<(), StrategyError> {
        if e.storage().instance().has(&DataKey::Initialized) {
            return Err(StrategyError::AlreadyInitialized);
        }
        e.storage().instance().set(&DataKey::Initialized, &true);
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::Vault, &vault);
        e.storage().instance().set(&DataKey::Asset, &asset);
        e.storage().instance().set(&DataKey::Healthy, &true);

        let key = DataKey::Balance;
        e.storage().persistent().set(&key, &0i128);
        bump_persistent(&e, &key);

        bump_instance(&e);
        Ok(())
    }

    // ----- Core operations (vault-only) -----

    /// Vault pre-transfers tokens before calling deposit.
    pub fn deposit(e: Env, caller: Address, amount: i128) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;
        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        // Tokens already received from vault (pre-transfer pattern).
        // Just update tracked balance.
        let key = DataKey::Balance;
        let balance: i128 = e.storage().persistent().get(&key).unwrap_or(0);
        e.storage().persistent().set(&key, &(balance + amount));
        bump_persistent(&e, &key);

        DepositEvent { amount }.publish(&e);
        bump_instance(&e);

        Ok(())
    }

    pub fn withdraw(e: Env, caller: Address, amount: i128) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;
        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let key = DataKey::Balance;
        let balance: i128 = e.storage().persistent().get(&key).unwrap_or(0);
        let withdraw_amount = if amount > balance { balance } else { amount };
        if withdraw_amount == 0 {
            return Err(StrategyError::InsufficientBalance);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&e, &asset).transfer(
            &e.current_contract_address(),
            &caller,
            &withdraw_amount,
        );

        e.storage()
            .persistent()
            .set(&key, &(balance - withdraw_amount));
        bump_persistent(&e, &key);

        WithdrawEvent {
            amount: withdraw_amount,
        }
        .publish(&e);
        bump_instance(&e);

        Ok(withdraw_amount)
    }

    pub fn emergency_withdraw(e: Env, caller: Address) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;

        let key = DataKey::Balance;
        let balance: i128 = e.storage().persistent().get(&key).unwrap_or(0);
        if balance == 0 {
            return Ok(0);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&e, &asset).transfer(&e.current_contract_address(), &caller, &balance);

        e.storage().persistent().set(&key, &0i128);
        bump_persistent(&e, &key);

        EmergencyWithdrawEvent { amount: balance }.publish(&e);
        bump_instance(&e);

        Ok(balance)
    }

    // ----- View functions -----

    pub fn balance_of(e: Env) -> i128 {
        e.storage().persistent().get(&DataKey::Balance).unwrap_or(0)
    }

    pub fn available_liquidity(e: Env) -> i128 {
        Self::balance_of(e)
    }

    pub fn is_healthy(e: Env) -> bool {
        e.storage()
            .instance()
            .get(&DataKey::Healthy)
            .unwrap_or(true)
    }

    pub fn asset(e: Env) -> Address {
        e.storage().instance().get(&DataKey::Asset).unwrap()
    }

    pub fn vault(e: Env) -> Address {
        e.storage().instance().get(&DataKey::Vault).unwrap()
    }

    // ----- Admin test helpers -----

    /// Simulate yield by increasing tracked balance.
    /// Caller must also mint/transfer actual tokens to match.
    pub fn simulate_yield(e: Env, caller: Address, amount: i128) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;

        let key = DataKey::Balance;
        let balance: i128 = e.storage().persistent().get(&key).unwrap_or(0);
        e.storage().persistent().set(&key, &(balance + amount));
        bump_persistent(&e, &key);

        Ok(())
    }

    /// Toggle health status for testing.
    pub fn set_healthy(e: Env, caller: Address, healthy: bool) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        e.storage().instance().set(&DataKey::Healthy, &healthy);
        Ok(())
    }

    // ----- Internal -----

    fn require_vault(e: &Env, caller: &Address) -> Result<(), StrategyError> {
        let vault: Address = e.storage().instance().get(&DataKey::Vault).unwrap();
        if *caller != vault {
            return Err(StrategyError::Unauthorized);
        }
        Ok(())
    }

    fn require_admin(e: &Env, caller: &Address) -> Result<(), StrategyError> {
        let admin: Address = e.storage().instance().get(&DataKey::Admin).unwrap();
        if *caller != admin {
            return Err(StrategyError::Unauthorized);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::token::{StellarAssetClient, TokenClient};

    #[test]
    fn test_deposit_withdraw_cycle() {
        let e = Env::default();
        e.mock_all_auths();

        let admin = Address::generate(&e);
        let vault = Address::generate(&e);
        let token_admin = Address::generate(&e);
        let token_contract = e.register_stellar_asset_contract_v2(token_admin.clone());
        let asset = token_contract.address();

        StellarAssetClient::new(&e, &asset).mint(&vault, &10_000_0000000);

        let contract_id = e.register(MockStrategy, ());
        let client = MockStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset);

        // Pre-transfer + deposit (matches vault.allocate flow)
        let token = TokenClient::new(&e, &asset);
        token.transfer(&vault, &contract_id, &1000_0000000);
        client.deposit(&vault, &1000_0000000);
        assert_eq!(client.balance_of(), 1000_0000000);
        assert_eq!(token.balance(&contract_id), 1000_0000000);

        // Simulate yield: mint tokens + update tracked balance
        let stellar_client = StellarAssetClient::new(&e, &asset);
        stellar_client.mint(&contract_id, &50_0000000);
        client.simulate_yield(&admin, &50_0000000);
        assert_eq!(client.balance_of(), 1050_0000000);
        assert_eq!(token.balance(&contract_id), 1050_0000000);

        // Withdraw all
        let withdrawn = client.withdraw(&vault, &1050_0000000);
        assert_eq!(withdrawn, 1050_0000000);
        assert_eq!(client.balance_of(), 0);
    }

    #[test]
    fn test_health_toggle() {
        let e = Env::default();
        e.mock_all_auths();

        let admin = Address::generate(&e);
        let vault = Address::generate(&e);
        let asset = Address::generate(&e);

        let contract_id = e.register(MockStrategy, ());
        let client = MockStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset);

        assert!(client.is_healthy());
        client.set_healthy(&admin, &false);
        assert!(!client.is_healthy());
    }

    #[test]
    fn test_emergency_withdraw() {
        let e = Env::default();
        e.mock_all_auths();

        let admin = Address::generate(&e);
        let vault = Address::generate(&e);
        let token_admin = Address::generate(&e);
        let token_contract = e.register_stellar_asset_contract_v2(token_admin.clone());
        let asset = token_contract.address();

        StellarAssetClient::new(&e, &asset).mint(&vault, &10_000_0000000);

        let contract_id = e.register(MockStrategy, ());
        let client = MockStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset);
        TokenClient::new(&e, &asset).transfer(&vault, &contract_id, &500_0000000);
        client.deposit(&vault, &500_0000000);

        let withdrawn = client.emergency_withdraw(&vault);
        assert_eq!(withdrawn, 500_0000000);
        assert_eq!(client.balance_of(), 0);
    }

    // -----------------------------------------------------------------------
    // Error-path coverage
    // -----------------------------------------------------------------------

    fn init_default<'a>(e: &Env) -> (MockStrategyClient<'a>, Address, Address, Address) {
        let admin = Address::generate(e);
        let vault = Address::generate(e);
        let asset = Address::generate(e);
        let contract_id = e.register(MockStrategy, ());
        let client = MockStrategyClient::new(e, &contract_id);
        client.initialize(&admin, &vault, &asset);
        (client, admin, vault, asset)
    }

    #[test]
    fn test_double_initialize_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, admin, vault, asset) = init_default(&e);
        assert!(client.try_initialize(&admin, &vault, &asset).is_err());
    }

    #[test]
    fn test_deposit_zero_amount_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, vault, _) = init_default(&e);
        assert!(client.try_deposit(&vault, &0).is_err());
    }

    #[test]
    fn test_withdraw_zero_amount_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, vault, _) = init_default(&e);
        assert!(client.try_withdraw(&vault, &0).is_err());
    }

    #[test]
    fn test_withdraw_with_zero_balance_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, vault, _) = init_default(&e);
        // Nothing deposited — withdraw must fail with InsufficientBalance.
        assert!(client.try_withdraw(&vault, &100).is_err());
    }

    #[test]
    fn test_emergency_withdraw_empty_returns_zero() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, vault, _) = init_default(&e);
        // emergency_withdraw against an empty strategy returns 0 without a
        // token transfer — exercises the `balance == 0` short-circuit.
        assert_eq!(client.emergency_withdraw(&vault), 0);
    }

    #[test]
    fn test_deposit_unauthorized_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _) = init_default(&e);
        let stranger = Address::generate(&e);
        assert!(client.try_deposit(&stranger, &100).is_err());
    }

    #[test]
    fn test_simulate_yield_requires_admin() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _) = init_default(&e);
        let stranger = Address::generate(&e);
        assert!(client.try_simulate_yield(&stranger, &100).is_err());
    }

    #[test]
    fn test_view_getters() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, vault, asset) = init_default(&e);
        assert_eq!(client.asset(), asset);
        assert_eq!(client.vault(), vault);
        assert_eq!(client.available_liquidity(), 0);
    }
}
