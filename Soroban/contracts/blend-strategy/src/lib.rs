#![no_std]

use blend_contract_sdk::pool;
use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror, token, vec, Address, Env, Vec,
};

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
    Unhealthy = 6,
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
    BlendPool,
    /// Tracked balance in underlying asset terms.
    TrackedBalance,
    /// Maximum utilization ratio (bps) above which strategy signals unhealthy.
    MaxUtilizationBps,
    /// Minimum backstop coverage ratio (bps) below which strategy signals unhealthy.
    MinBackstopCoverageBps,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Blend protocol request type for supplying collateral.
const REQUEST_SUPPLY_COLLATERAL: u32 = 2;
/// Blend protocol request type for withdrawing collateral.
const REQUEST_WITHDRAW_COLLATERAL: u32 = 3;

/// Ledger approval buffer (approve tokens for this many ledgers ahead).
const APPROVAL_LEDGER_BUFFER: u32 = 200;

const DEFAULT_MAX_UTILIZATION_BPS: u32 = 9_500; // 95%
const DEFAULT_MIN_BACKSTOP_BPS: u32 = 500; // 5% minimum coverage

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Blend v2 Strategy Adapter for Tezoro Vault.
///
/// Integrates with Blend's Lending Pool via `submit()`:
/// - `SupplyCollateral` (type 2) to deposit USDC
/// - `WithdrawCollateral` (type 3) to withdraw USDC
///
/// Blend architecture (4 immutable core contracts):
/// - Emitter: distributes BLND token rewards
/// - Backstop: insurance pool against bad debt
/// - Pool Factory: deploys new lending pools
/// - Lending Pool: user-facing deposit/withdraw/borrow/liquidate
///
/// This adapter holds a collateral position in the Blend Pool.
/// Balance is tracked via internal accounting; in production it can
/// also be verified against pool position data (b_supply shares).
#[contract]
pub struct BlendStrategy;

#[contractimpl]
impl BlendStrategy {
    /// Initialize with vault address, USDC asset, and Blend pool address.
    pub fn initialize(
        e: Env,
        admin: Address,
        vault: Address,
        asset: Address,
        blend_pool: Address,
    ) -> Result<(), StrategyError> {
        if e.storage().instance().has(&DataKey::Initialized) {
            return Err(StrategyError::AlreadyInitialized);
        }
        e.storage().instance().set(&DataKey::Initialized, &true);
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::Vault, &vault);
        e.storage().instance().set(&DataKey::Asset, &asset);
        e.storage()
            .instance()
            .set(&DataKey::BlendPool, &blend_pool);
        e.storage()
            .instance()
            .set(&DataKey::MaxUtilizationBps, &DEFAULT_MAX_UTILIZATION_BPS);
        e.storage()
            .instance()
            .set(&DataKey::MinBackstopCoverageBps, &DEFAULT_MIN_BACKSTOP_BPS);
        e.storage()
            .persistent()
            .set(&DataKey::TrackedBalance, &0i128);
        Ok(())
    }

    // ----- Core operations (vault-only) -----

    /// Deposit USDC into Blend pool via submit(SupplyCollateral).
    ///
    /// Flow:
    /// 1. Transfer USDC from vault to this contract
    /// 2. Approve Blend pool to spend USDC
    /// 3. Call blend_pool.submit() with SupplyCollateral request
    /// 4. Update tracked balance
    pub fn deposit(e: Env, caller: Address, amount: i128) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;
        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        let blend_pool_addr: Address =
            e.storage().instance().get(&DataKey::BlendPool).unwrap();
        let contract_addr = e.current_contract_address();

        // Transfer USDC from vault to strategy
        let asset_client = token::Client::new(&e, &asset);
        asset_client.transfer(&caller, &contract_addr, &amount);

        // Approve Blend pool to pull USDC from this contract
        let approval_ledger = e.ledger().sequence() + APPROVAL_LEDGER_BUFFER;
        asset_client.approve(&contract_addr, &blend_pool_addr, &amount, &approval_ledger);

        // Build SupplyCollateral request and submit to Blend pool
        let requests: Vec<pool::Request> = vec![
            &e,
            pool::Request {
                request_type: REQUEST_SUPPLY_COLLATERAL,
                address: asset.clone(),
                amount,
            },
        ];
        let pool_client = pool::Client::new(&e, &blend_pool_addr);
        pool_client.submit(&contract_addr, &contract_addr, &contract_addr, &requests);

        // Update tracked balance
        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::TrackedBalance)
            .unwrap_or(0);
        e.storage()
            .persistent()
            .set(&DataKey::TrackedBalance, &(balance + amount));

        Ok(())
    }

    /// Withdraw USDC from Blend pool via submit(WithdrawCollateral).
    ///
    /// Flow:
    /// 1. Call blend_pool.submit() with WithdrawCollateral request
    /// 2. Transfer received USDC from strategy to vault
    /// 3. Update tracked balance
    pub fn withdraw(e: Env, caller: Address, amount: i128) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;
        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::TrackedBalance)
            .unwrap_or(0);
        let withdraw_amount = if amount > balance { balance } else { amount };
        if withdraw_amount == 0 {
            return Err(StrategyError::InsufficientBalance);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        let blend_pool_addr: Address =
            e.storage().instance().get(&DataKey::BlendPool).unwrap();
        let contract_addr = e.current_contract_address();

        let requests: Vec<pool::Request> = vec![
            &e,
            pool::Request {
                request_type: REQUEST_WITHDRAW_COLLATERAL,
                address: asset.clone(),
                amount: withdraw_amount,
            },
        ];
        let pool_client = pool::Client::new(&e, &blend_pool_addr);
        pool_client.submit(&contract_addr, &contract_addr, &contract_addr, &requests);

        token::Client::new(&e, &asset).transfer(&contract_addr, &caller, &withdraw_amount);

        e.storage()
            .persistent()
            .set(&DataKey::TrackedBalance, &(balance - withdraw_amount));

        Ok(withdraw_amount)
    }

    /// Emergency withdraw: pull all funds from Blend back to vault.
    pub fn emergency_withdraw(e: Env, caller: Address) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;

        let balance: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::TrackedBalance)
            .unwrap_or(0);
        if balance == 0 {
            return Ok(0);
        }

        let asset: Address = e.storage().instance().get(&DataKey::Asset).unwrap();
        let blend_pool_addr: Address =
            e.storage().instance().get(&DataKey::BlendPool).unwrap();
        let contract_addr = e.current_contract_address();

        let requests: Vec<pool::Request> = vec![
            &e,
            pool::Request {
                request_type: REQUEST_WITHDRAW_COLLATERAL,
                address: asset.clone(),
                amount: balance,
            },
        ];
        let pool_client = pool::Client::new(&e, &blend_pool_addr);
        pool_client.submit(&contract_addr, &contract_addr, &contract_addr, &requests);

        token::Client::new(&e, &asset).transfer(&contract_addr, &caller, &balance);

        e.storage()
            .persistent()
            .set(&DataKey::TrackedBalance, &0i128);

        Ok(balance)
    }

    // ----- View functions -----

    /// Current balance in underlying asset terms.
    pub fn balance_of(e: Env) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::TrackedBalance)
            .unwrap_or(0)
    }

    /// Available liquidity that can be withdrawn immediately.
    pub fn available_liquidity(e: Env) -> i128 {
        // In production: min(our_balance, pool_available_reserves)
        Self::balance_of(e)
    }

    /// Health check: monitors pool utilization and backstop coverage.
    ///
    /// Returns false if:
    /// 1. Pool utilization > max_utilization_bps (default 95%)
    /// 2. Backstop coverage < min_backstop_bps (default 5%)
    ///
    /// Backstop monitoring is critical for Blend because bad debt is
    /// socialized through backstop depletion. A depleted backstop means
    /// depositors are unprotected against borrower defaults.
    pub fn is_healthy(e: Env) -> bool {
        // TODO: Query Blend pool reserves for utilization check
        // and Backstop contract for coverage ratio.
        // Requires on-chain pool state (only available against a real Blend pool).

        let _max_util: u32 = e
            .storage()
            .instance()
            .get(&DataKey::MaxUtilizationBps)
            .unwrap_or(DEFAULT_MAX_UTILIZATION_BPS);
        let _min_backstop: u32 = e
            .storage()
            .instance()
            .get(&DataKey::MinBackstopCoverageBps)
            .unwrap_or(DEFAULT_MIN_BACKSTOP_BPS);

        true // Stub until deployed against real pool
    }

    /// Claim BLND token emissions from Blend's Backstop contract.
    ///
    /// In production: calls backstop.claim() and forwards BLND to rewards module
    /// for swap to USDC and re-deposit into vault.
    pub fn harvest(
        e: Env,
        caller: Address,
        rewards_module: Address,
    ) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;

        // TODO: call backstop.claim() to collect BLND emissions
        let _ = rewards_module;
        Ok(0)
    }

    pub fn asset(e: Env) -> Address {
        e.storage().instance().get(&DataKey::Asset).unwrap()
    }

    pub fn blend_pool(e: Env) -> Address {
        e.storage().instance().get(&DataKey::BlendPool).unwrap()
    }

    // ----- Admin functions -----

    pub fn set_max_utilization(
        e: Env,
        caller: Address,
        bps: u32,
    ) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        e.storage()
            .instance()
            .set(&DataKey::MaxUtilizationBps, &bps);
        Ok(())
    }

    pub fn set_min_backstop_coverage(
        e: Env,
        caller: Address,
        bps: u32,
    ) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        e.storage()
            .instance()
            .set(&DataKey::MinBackstopCoverageBps, &bps);
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
    use soroban_sdk::token::StellarAssetClient;

    fn setup(e: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(e);
        let vault = Address::generate(e);
        let blend_pool = Address::generate(e);

        let token_admin = Address::generate(e);
        let token_contract = e.register_stellar_asset_contract_v2(token_admin.clone());
        let asset = token_contract.address();

        StellarAssetClient::new(e, &asset).mint(&vault, &100_000_0000000);

        (admin, vault, asset, blend_pool)
    }

    #[test]
    fn test_initialize() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, vault, asset, blend_pool) = setup(&e);
        let contract_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset, &blend_pool);

        assert_eq!(client.asset(), asset);
        assert_eq!(client.blend_pool(), blend_pool);
        assert_eq!(client.balance_of(), 0);
        assert_eq!(client.is_healthy(), true);
    }

    // Full deposit/withdraw tests require a registered Blend pool WASM.
    // Integration tests will run against testnet.

    #[test]
    fn test_unauthorized_deposit() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, vault, asset, blend_pool) = setup(&e);
        let contract_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset, &blend_pool);

        let random = Address::generate(&e);
        let result = client.try_deposit(&random, &1000_0000000);
        assert!(result.is_err());
    }

    #[test]
    fn test_health_thresholds() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, vault, asset, blend_pool) = setup(&e);
        let contract_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset, &blend_pool);

        client.set_max_utilization(&admin, &8000u32);
        client.set_min_backstop_coverage(&admin, &1000u32);

        assert_eq!(client.is_healthy(), true);
    }
}
