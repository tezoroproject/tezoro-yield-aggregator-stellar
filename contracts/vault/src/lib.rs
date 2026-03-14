#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, token, Address, Env, Vec,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
    ZeroAmount = 5,
    InsufficientShares = 6,
    InsufficientAssets = 7,
    DepositCapExceeded = 8,
    MaxStrategies = 9,
    StrategyAlreadyActive = 10,
    StrategyNotActive = 11,
    InvalidBps = 12,
    AllocationMismatch = 13,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    // Instance storage (contract-lifetime)
    Admin,
    Keeper,
    Guardian,
    Asset,
    Initialized,
    Paused,
    PerformanceFeeBps,
    FeeRecipient,
    IdleBufferBps,
    MaxDeviationBps,
    DepositCap,
    StrategyList,

    // Persistent storage (survives archival)
    TotalSupply,
    HighWaterMark,
    TrackedBalance(Address),
    ShareBalance(Address),

    // Bridged balance attestation (cross-chain accounting)
    BridgedBalance,
    BridgedTimestamp,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_STRATEGIES: u32 = 20;
const DECIMALS: u32 = 7; // Stellar USDC uses 7 decimals
const VIRTUAL_SHARES_OFFSET: i128 = 1_000_000; // 10^6 offset to prevent inflation attack

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct TezoroVault;

#[contractimpl]
impl TezoroVault {
    // -----------------------------------------------------------------------
    // Initialization
    // -----------------------------------------------------------------------

    /// Initialize the vault with admin, asset token, and fee configuration.
    pub fn initialize(
        e: Env,
        admin: Address,
        asset: Address,
        keeper: Address,
        guardian: Address,
        fee_recipient: Address,
        performance_fee_bps: u32,
        idle_buffer_bps: u32,
    ) -> Result<(), VaultError> {
        if e.storage().instance().has(&DataKey::Initialized) {
            return Err(VaultError::AlreadyInitialized);
        }
        if performance_fee_bps > 3_000 {
            return Err(VaultError::InvalidBps);
        }
        if idle_buffer_bps > 2_000 {
            return Err(VaultError::InvalidBps);
        }

        e.storage().instance().set(&DataKey::Initialized, &true);
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::Keeper, &keeper);
        e.storage().instance().set(&DataKey::Guardian, &guardian);
        e.storage().instance().set(&DataKey::Asset, &asset);
        e.storage()
            .instance()
            .set(&DataKey::FeeRecipient, &fee_recipient);
        e.storage()
            .instance()
            .set(&DataKey::PerformanceFeeBps, &performance_fee_bps);
        e.storage()
            .instance()
            .set(&DataKey::IdleBufferBps, &idle_buffer_bps);
        e.storage().instance().set(&DataKey::Paused, &false);
        e.storage()
            .instance()
            .set(&DataKey::MaxDeviationBps, &0u32);
        e.storage().instance().set(&DataKey::DepositCap, &0i128);
        e.storage()
            .instance()
            .set(&DataKey::StrategyList, &Vec::<Address>::new(&e));

        e.storage().persistent().set(&DataKey::TotalSupply, &0i128);
        e.storage()
            .persistent()
            .set(&DataKey::HighWaterMark, &0i128);
        e.storage()
            .persistent()
            .set(&DataKey::BridgedBalance, &0i128);
        e.storage()
            .persistent()
            .set(&DataKey::BridgedTimestamp, &0u64);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // SEP-41 Token Interface (vault shares)
    // -----------------------------------------------------------------------

    pub fn name(e: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&e, "Tezoro USDC-A")
    }

    pub fn symbol(e: Env) -> soroban_sdk::String {
        soroban_sdk::String::from_str(&e, "tUSDC-A")
    }

    pub fn decimals(_e: Env) -> u32 {
        DECIMALS
    }

    pub fn balance(e: Env, id: Address) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::ShareBalance(id))
            .unwrap_or(0)
    }

    pub fn total_supply(e: Env) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) -> Result<(), VaultError> {
        from.require_auth();
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let from_bal = Self::balance(e.clone(), from.clone());
        if from_bal < amount {
            return Err(VaultError::InsufficientShares);
        }
        e.storage()
            .persistent()
            .set(&DataKey::ShareBalance(from), &(from_bal - amount));
        let to_bal = Self::balance(e.clone(), to.clone());
        e.storage()
            .persistent()
            .set(&DataKey::ShareBalance(to), &(to_bal + amount));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Vault Core
    // -----------------------------------------------------------------------

    /// Total assets under management: idle balance + strategy balances + bridged balance.
    pub fn total_assets(e: Env) -> i128 {
        let asset_addr: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        let idle = token::Client::new(&e, &asset_addr).balance(&e.current_contract_address());

        let strategies: Vec<Address> = e
            .storage()
            .instance()
            .get(&DataKey::StrategyList)
            .unwrap_or(Vec::new(&e));

        let mut strategy_total: i128 = 0;
        for s in strategies.iter() {
            let tracked: i128 = e
                .storage()
                .persistent()
                .get(&DataKey::TrackedBalance(s))
                .unwrap_or(0);
            strategy_total += tracked;
        }

        let bridged: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::BridgedBalance)
            .unwrap_or(0);

        idle + strategy_total + bridged
    }

    /// Convert asset amount to shares using virtual offset (prevents inflation attack).
    pub fn convert_to_shares(e: Env, assets: i128) -> i128 {
        let total_assets = Self::total_assets(e.clone());
        let total_supply = Self::total_supply(e);
        let adjusted_assets = total_assets + VIRTUAL_SHARES_OFFSET;
        let adjusted_supply = total_supply + VIRTUAL_SHARES_OFFSET;
        assets * adjusted_supply / adjusted_assets
    }

    /// Convert shares to asset amount using virtual offset.
    pub fn convert_to_assets(e: Env, shares: i128) -> i128 {
        let total_assets = Self::total_assets(e.clone());
        let total_supply = Self::total_supply(e);
        let adjusted_assets = total_assets + VIRTUAL_SHARES_OFFSET;
        let adjusted_supply = total_supply + VIRTUAL_SHARES_OFFSET;
        shares * adjusted_assets / adjusted_supply
    }

    /// Deposit assets into the vault, receive shares.
    pub fn deposit(e: Env, from: Address, assets: i128) -> Result<i128, VaultError> {
        from.require_auth();
        Self::require_not_paused(&e)?;
        if assets <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        // Check deposit cap (0 = unlimited)
        let cap: i128 = e
            .storage()
            .instance()
            .get(&DataKey::DepositCap)
            .unwrap_or(0);
        if cap > 0 {
            let current = Self::total_assets(e.clone());
            if current + assets > cap {
                return Err(VaultError::DepositCapExceeded);
            }
        }

        let shares = Self::convert_to_shares(e.clone(), assets);
        if shares <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        // Transfer asset tokens from depositor to vault
        let asset_addr: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        token::Client::new(&e, &asset_addr).transfer(
            &from,
            &e.current_contract_address(),
            &assets,
        );

        // Mint shares
        let current_shares = Self::balance(e.clone(), from.clone());
        e.storage()
            .persistent()
            .set(&DataKey::ShareBalance(from), &(current_shares + shares));

        let supply = Self::total_supply(e.clone());
        e.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &(supply + shares));

        Ok(shares)
    }

    /// Redeem shares for underlying assets.
    ///
    /// Only serves from idle buffer -- keeper pre-ensures sufficient idle liquidity.
    /// If idle is insufficient, the transaction reverts.
    pub fn redeem(e: Env, from: Address, shares: i128) -> Result<i128, VaultError> {
        from.require_auth();
        if shares <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let current_shares = Self::balance(e.clone(), from.clone());
        if current_shares < shares {
            return Err(VaultError::InsufficientShares);
        }

        let assets = Self::convert_to_assets(e.clone(), shares);
        if assets <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let asset_addr: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        let idle = token::Client::new(&e, &asset_addr).balance(&e.current_contract_address());
        if idle < assets {
            return Err(VaultError::InsufficientAssets);
        }

        // Burn shares
        e.storage().persistent().set(
            &DataKey::ShareBalance(from.clone()),
            &(current_shares - shares),
        );
        let supply = Self::total_supply(e.clone());
        e.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &(supply - shares));

        // Transfer assets to redeemer
        token::Client::new(&e, &asset_addr).transfer(
            &e.current_contract_address(),
            &from,
            &assets,
        );

        Ok(assets)
    }

    // -----------------------------------------------------------------------
    // Strategy Management (admin-only)
    // -----------------------------------------------------------------------

    pub fn add_strategy(e: Env, caller: Address, strategy: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;

        let mut strategies: Vec<Address> = e
            .storage()
            .instance()
            .get(&DataKey::StrategyList)
            .unwrap_or(Vec::new(&e));

        if strategies.len() >= MAX_STRATEGIES {
            return Err(VaultError::MaxStrategies);
        }

        for s in strategies.iter() {
            if s == strategy {
                return Err(VaultError::StrategyAlreadyActive);
            }
        }

        strategies.push_back(strategy.clone());
        e.storage()
            .instance()
            .set(&DataKey::StrategyList, &strategies);
        e.storage()
            .persistent()
            .set(&DataKey::TrackedBalance(strategy), &0i128);

        Ok(())
    }

    pub fn remove_strategy(
        e: Env,
        caller: Address,
        strategy: Address,
    ) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;

        let strategies: Vec<Address> = e
            .storage()
            .instance()
            .get(&DataKey::StrategyList)
            .unwrap_or(Vec::new(&e));

        let mut found = false;
        let mut new_strategies = Vec::new(&e);
        for s in strategies.iter() {
            if s == strategy {
                found = true;
            } else {
                new_strategies.push_back(s);
            }
        }
        if !found {
            return Err(VaultError::StrategyNotActive);
        }

        e.storage()
            .instance()
            .set(&DataKey::StrategyList, &new_strategies);
        e.storage()
            .persistent()
            .remove(&DataKey::TrackedBalance(strategy));

        Ok(())
    }

    pub fn get_strategies(e: Env) -> Vec<Address> {
        e.storage()
            .instance()
            .get(&DataKey::StrategyList)
            .unwrap_or(Vec::new(&e))
    }

    // -----------------------------------------------------------------------
    // Keeper Operations
    // -----------------------------------------------------------------------

    /// Update the bridged balance attestation (keeper-only).
    ///
    /// This is the oracle-attested balance from EVM chains. The keeper signs
    /// a periodic attestation reporting total EVM-side principal + accrued yield.
    /// `total_assets()` includes this value so share price reflects cross-chain capital.
    pub fn attest_bridged_balance(
        e: Env,
        keeper: Address,
        balance: i128,
    ) -> Result<(), VaultError> {
        keeper.require_auth();
        Self::require_keeper(&e, &keeper)?;

        e.storage()
            .persistent()
            .set(&DataKey::BridgedBalance, &balance);
        e.storage()
            .persistent()
            .set(&DataKey::BridgedTimestamp, &e.ledger().timestamp());

        Ok(())
    }

    /// Update tracked balance for a strategy after rebalance (keeper-only).
    pub fn update_tracked_balance(
        e: Env,
        keeper: Address,
        strategy: Address,
        balance: i128,
    ) -> Result<(), VaultError> {
        keeper.require_auth();
        Self::require_keeper(&e, &keeper)?;

        let strategies: Vec<Address> = e
            .storage()
            .instance()
            .get(&DataKey::StrategyList)
            .unwrap_or(Vec::new(&e));

        let mut found = false;
        for s in strategies.iter() {
            if s == strategy {
                found = true;
                break;
            }
        }
        if !found {
            return Err(VaultError::StrategyNotActive);
        }

        e.storage()
            .persistent()
            .set(&DataKey::TrackedBalance(strategy), &balance);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Admin Operations
    // -----------------------------------------------------------------------

    /// Pause deposits. Both guardian and admin can pause.
    pub fn pause(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        let admin: Address = e.storage().instance().get(&DataKey::Admin).unwrap();
        let guardian: Address = e.storage().instance().get(&DataKey::Guardian).unwrap();
        if caller != admin && caller != guardian {
            return Err(VaultError::Unauthorized);
        }
        e.storage().instance().set(&DataKey::Paused, &true);
        Ok(())
    }

    /// Unpause deposits. Admin-only.
    pub fn unpause(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        e.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    pub fn set_deposit_cap(e: Env, caller: Address, cap: i128) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        e.storage().instance().set(&DataKey::DepositCap, &cap);
        Ok(())
    }

    pub fn set_keeper(e: Env, caller: Address, new_keeper: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        e.storage().instance().set(&DataKey::Keeper, &new_keeper);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // View Functions
    // -----------------------------------------------------------------------

    pub fn asset(e: Env) -> Address {
        e.storage().instance().get(&DataKey::Asset).unwrap()
    }

    pub fn admin(e: Env) -> Address {
        e.storage().instance().get(&DataKey::Admin).unwrap()
    }

    pub fn keeper(e: Env) -> Address {
        e.storage().instance().get(&DataKey::Keeper).unwrap()
    }

    pub fn is_paused(e: Env) -> bool {
        e.storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    pub fn bridged_balance(e: Env) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::BridgedBalance)
            .unwrap_or(0)
    }

    pub fn bridged_timestamp(e: Env) -> u64 {
        e.storage()
            .persistent()
            .get(&DataKey::BridgedTimestamp)
            .unwrap_or(0)
    }

    pub fn tracked_balance(e: Env, strategy: Address) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::TrackedBalance(strategy))
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Internal Helpers
    // -----------------------------------------------------------------------

    fn require_admin(e: &Env, caller: &Address) -> Result<(), VaultError> {
        let admin: Address = e.storage().instance().get(&DataKey::Admin).unwrap();
        if *caller != admin {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }

    fn require_keeper(e: &Env, caller: &Address) -> Result<(), VaultError> {
        let keeper: Address = e.storage().instance().get(&DataKey::Keeper).unwrap();
        if *caller != keeper {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }

    fn require_not_paused(e: &Env) -> Result<(), VaultError> {
        let paused: bool = e
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if paused {
            return Err(VaultError::Paused);
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

    fn setup_test(e: &Env) -> (Address, Address, Address, Address, Address, Address) {
        let admin = Address::generate(e);
        let keeper = Address::generate(e);
        let guardian = Address::generate(e);
        let fee_recipient = Address::generate(e);
        let user = Address::generate(e);

        let token_admin = Address::generate(e);
        let token_contract = e.register_stellar_asset_contract_v2(token_admin.clone());
        let asset = token_contract.address();

        // Mint 10,000 USDC (7 decimals)
        StellarAssetClient::new(e, &asset).mint(&user, &10_000_0000000);

        (admin, keeper, guardian, fee_recipient, user, asset)
    }

    #[test]
    fn test_initialize() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, keeper, guardian, fee_recipient, _user, asset) = setup_test(&e);
        let contract_id = e.register(TezoroVault, ());
        let client = TezoroVaultClient::new(&e, &contract_id);

        client.initialize(
            &admin,
            &asset,
            &keeper,
            &guardian,
            &fee_recipient,
            &1500u32, // 15% performance fee
            &300u32,  // 3% idle buffer
        );

        assert_eq!(client.admin(), admin);
        assert_eq!(client.keeper(), keeper);
        assert_eq!(client.asset(), asset);
        assert_eq!(client.total_supply(), 0);
        assert_eq!(client.total_assets(), 0);
        assert_eq!(client.is_paused(), false);
    }

    #[test]
    fn test_deposit_and_redeem() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, keeper, guardian, fee_recipient, user, asset) = setup_test(&e);
        let contract_id = e.register(TezoroVault, ());
        let client = TezoroVaultClient::new(&e, &contract_id);

        client.initialize(&admin, &asset, &keeper, &guardian, &fee_recipient, &1500u32, &300u32);

        // Deposit 1000 USDC
        let deposit_amount: i128 = 1000_0000000;
        let shares = client.deposit(&user, &deposit_amount);
        assert!(shares > 0);
        assert_eq!(client.total_supply(), shares);
        assert_eq!(client.balance(&user), shares);

        let token = TokenClient::new(&e, &asset);
        assert_eq!(token.balance(&contract_id), deposit_amount);

        // Redeem all shares
        let withdrawn = client.redeem(&user, &shares);
        assert_eq!(withdrawn, deposit_amount);
        assert_eq!(client.total_supply(), 0);
        assert_eq!(client.balance(&user), 0);
    }

    #[test]
    fn test_strategy_management() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, keeper, guardian, fee_recipient, _user, asset) = setup_test(&e);
        let contract_id = e.register(TezoroVault, ());
        let client = TezoroVaultClient::new(&e, &contract_id);

        client.initialize(&admin, &asset, &keeper, &guardian, &fee_recipient, &1500u32, &300u32);

        let strategy = Address::generate(&e);
        client.add_strategy(&admin, &strategy);

        let strategies = client.get_strategies();
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies.get(0).unwrap(), strategy);

        // Update tracked balance via keeper
        client.update_tracked_balance(&keeper, &strategy, &500_0000000);
        assert_eq!(client.tracked_balance(&strategy), 500_0000000);

        // Remove strategy
        client.remove_strategy(&admin, &strategy);
        assert_eq!(client.get_strategies().len(), 0);
    }

    #[test]
    fn test_pause_unpause() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, _keeper, guardian, fee_recipient, user, asset) = setup_test(&e);
        let contract_id = e.register(TezoroVault, ());
        let client = TezoroVaultClient::new(&e, &contract_id);

        client.initialize(&admin, &asset, &_keeper, &guardian, &fee_recipient, &1500u32, &300u32);

        // Guardian can pause
        client.pause(&guardian);
        assert_eq!(client.is_paused(), true);

        // Deposit should fail when paused
        let result = client.try_deposit(&user, &100_0000000);
        assert!(result.is_err());

        // Admin can unpause
        client.unpause(&admin);
        assert_eq!(client.is_paused(), false);

        // Deposit should work again
        let result = client.try_deposit(&user, &100_0000000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_bridged_balance_attestation() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, keeper, guardian, fee_recipient, user, asset) = setup_test(&e);
        let contract_id = e.register(TezoroVault, ());
        let client = TezoroVaultClient::new(&e, &contract_id);

        client.initialize(&admin, &asset, &keeper, &guardian, &fee_recipient, &1500u32, &300u32);

        client.deposit(&user, &1000_0000000);

        // Keeper attests bridged balance (EVM-side capital)
        client.attest_bridged_balance(&keeper, &500_0000000);
        assert_eq!(client.bridged_balance(), 500_0000000);

        // Total assets should include bridged balance
        let total = client.total_assets();
        assert_eq!(total, 1000_0000000 + 500_0000000);
    }
}
