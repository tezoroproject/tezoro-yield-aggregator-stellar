#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, contracterror, token, Address, Env};

/// Mock strategy for testing vault integration.
///
/// Simulates a lending protocol by holding tokens and allowing
/// the admin to set a fake yield (increase tracked balance).

// ---------------------------------------------------------------------------
// Errors
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
        e.storage().persistent().set(&DataKey::Balance, &0i128);
        Ok(())
    }

    // ----- Core operations (vault-only) -----

    pub fn deposit(e: Env, caller: Address, amount: i128) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;
        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&e, &asset).transfer(&caller, &e.current_contract_address(), &amount);

        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::Balance)
            .unwrap_or(0);
        e.storage()
            .persistent()
            .set(&DataKey::Balance, &(balance + amount));

        Ok(())
    }

    pub fn withdraw(e: Env, caller: Address, amount: i128) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;
        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::Balance)
            .unwrap_or(0);
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
            .set(&DataKey::Balance, &(balance - withdraw_amount));

        Ok(withdraw_amount)
    }

    pub fn emergency_withdraw(e: Env, caller: Address) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;

        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::Balance)
            .unwrap_or(0);
        if balance == 0 {
            return Ok(0);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&e, &asset).transfer(
            &e.current_contract_address(),
            &caller,
            &balance,
        );

        e.storage().persistent().set(&DataKey::Balance, &0i128);
        Ok(balance)
    }

    // ----- View functions -----

    pub fn balance_of(e: Env) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::Balance)
            .unwrap_or(0)
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

        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::Balance)
            .unwrap_or(0);
        e.storage()
            .persistent()
            .set(&DataKey::Balance, &(balance + amount));
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

        // Deposit
        client.deposit(&vault, &1000_0000000);
        assert_eq!(client.balance_of(), 1000_0000000);

        let token = TokenClient::new(&e, &asset);
        assert_eq!(token.balance(&contract_id), 1000_0000000);

        // Simulate yield: mint tokens to contract + update tracked balance
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

        assert_eq!(client.is_healthy(), true);
        client.set_healthy(&admin, &false);
        assert_eq!(client.is_healthy(), false);
    }
}
