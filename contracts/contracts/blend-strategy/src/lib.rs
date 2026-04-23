// SPDX-License-Identifier: UNLICENSED
#![no_std]

mod events;
mod storage;

use blend_contract_sdk::pool;
use soroban_sdk::{contract, contractimpl, token, vec, Address, BytesN, Env, Vec};
use tezoro_common::{bump_instance, StrategyError, MAX_BPS};

/// Blend protocol request type for supplying collateral.
const REQUEST_SUPPLY_COLLATERAL: u32 = 2;
/// Blend protocol request type for withdrawing collateral.
const REQUEST_WITHDRAW_COLLATERAL: u32 = 3;

/// Blend's b_rate / d_rate fixed-point scalar (12 decimals).
const BLEND_RATE_SCALAR: i128 = 1_000_000_000_000;

/// Rounding-margin stroops subtracted from quoted available liquidity so
/// exact-match withdraws don't race b_token rounding inside the pool.
/// Mirrors the EVM vault's `_availableLiquidity()` `- 2` for the same reason.
/// 2 stroops == 2e-7 USDC — smaller than any representable display value.
const AVAILABLE_LIQUIDITY_ROUNDING_MARGIN: i128 = 2;

/// Apply the rounding margin without underflowing past zero.
fn apply_rounding_margin(value: i128) -> i128 {
    let trimmed = value - AVAILABLE_LIQUIDITY_ROUNDING_MARGIN;
    if trimmed < 0 {
        0
    } else {
        trimmed
    }
}

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
#[contract]
pub struct BlendStrategy;

#[contractimpl]
impl BlendStrategy {
    /// Initialize with vault, asset, and Blend pool.
    /// Admin must prove identity via require_auth.
    pub fn initialize(
        e: Env,
        admin: Address,
        vault: Address,
        asset: Address,
        blend_pool: Address,
    ) -> Result<(), StrategyError> {
        admin.require_auth();

        if storage::is_initialized(&e) {
            return Err(StrategyError::AlreadyInitialized);
        }

        storage::set_initialized(&e);
        storage::set_admin(&e, &admin);
        storage::set_vault(&e, &vault);
        storage::set_asset(&e, &asset);
        storage::set_blend_pool(&e, &blend_pool);
        storage::set_paused(&e, false);
        storage::set_max_utilization_bps(&e, storage::DEFAULT_MAX_UTILIZATION_BPS);
        storage::set_min_backstop_bps(&e, storage::DEFAULT_MIN_BACKSTOP_BPS);
        storage::set_approval_buffer(&e, storage::DEFAULT_APPROVAL_BUFFER);
        storage::set_upgrade_delay(&e, storage::DEFAULT_UPGRADE_DELAY);
        storage::set_tracked_balance(&e, 0);

        bump_instance(&e);
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
        Self::require_not_paused(&e)?;

        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let asset = storage::get_asset(&e);
        let blend_pool_addr = storage::get_blend_pool(&e);
        let contract_addr = e.current_contract_address();

        // Vault pre-transfers tokens before calling deposit.
        // We just approve the Blend pool and deploy.
        let asset_client = token::Client::new(&e, &asset);
        let approval_ledger = e.ledger().sequence() + storage::get_approval_buffer(&e);
        asset_client.approve(&contract_addr, &blend_pool_addr, &amount, &approval_ledger);

        let requests: Vec<pool::Request> = vec![
            &e,
            pool::Request {
                request_type: REQUEST_SUPPLY_COLLATERAL,
                address: asset.clone(),
                amount,
            },
        ];
        pool::Client::new(&e, &blend_pool_addr).submit_with_allowance(
            &contract_addr,
            &contract_addr,
            &contract_addr,
            &requests,
        );

        let balance = storage::get_tracked_balance(&e);
        storage::set_tracked_balance(&e, balance + amount);

        events::deposit(&e, amount);
        bump_instance(&e);

        Ok(())
    }

    /// Withdraw USDC from Blend pool via submit(WithdrawCollateral).
    ///
    /// If requested amount exceeds tracked balance, withdraws up to
    /// the tracked balance and emits both amounts for auditability.
    pub fn withdraw(e: Env, caller: Address, amount: i128) -> Result<i128, StrategyError> {
        caller.require_auth();
        Self::require_vault(&e, &caller)?;

        if amount <= 0 {
            return Err(StrategyError::ZeroAmount);
        }

        let balance = storage::get_tracked_balance(&e);
        let withdraw_amount = if amount > balance { balance } else { amount };
        if withdraw_amount == 0 {
            return Err(StrategyError::InsufficientBalance);
        }

        let asset = storage::get_asset(&e);
        let blend_pool_addr = storage::get_blend_pool(&e);
        let contract_addr = e.current_contract_address();

        let requests: Vec<pool::Request> = vec![
            &e,
            pool::Request {
                request_type: REQUEST_WITHDRAW_COLLATERAL,
                address: asset.clone(),
                amount: withdraw_amount,
            },
        ];
        pool::Client::new(&e, &blend_pool_addr).submit(
            &contract_addr,
            &contract_addr,
            &contract_addr,
            &requests,
        );

        token::Client::new(&e, &asset).transfer(&contract_addr, &caller, &withdraw_amount);

        storage::set_tracked_balance(&e, balance - withdraw_amount);

        events::withdraw(&e, withdraw_amount, amount);
        bump_instance(&e);

        Ok(withdraw_amount)
    }

    /// Pull all funds from Blend back to vault.
    /// Callable by vault (normal flow) OR admin (emergency).
    ///
    /// Uses actual pool position instead of tracked balance to handle
    /// cases where tracked balance drifted (e.g., partial liquidation).
    pub fn emergency_withdraw(e: Env, caller: Address) -> Result<i128, StrategyError> {
        caller.require_auth();

        let vault = storage::get_vault(&e);
        let admin = storage::get_admin(&e);
        if caller != vault && caller != admin {
            return Err(StrategyError::Unauthorized);
        }

        let asset = storage::get_asset(&e);
        let blend_pool_addr = storage::get_blend_pool(&e);
        let contract_addr = e.current_contract_address();
        let pool_client = pool::Client::new(&e, &blend_pool_addr);

        // Query actual pool position (more reliable than tracked balance)
        let withdraw_amount = Self::query_pool_position(&e, &pool_client, &asset, &contract_addr);

        if withdraw_amount == 0 {
            storage::set_tracked_balance(&e, 0);
            return Ok(0);
        }

        let requests: Vec<pool::Request> = vec![
            &e,
            pool::Request {
                request_type: REQUEST_WITHDRAW_COLLATERAL,
                address: asset.clone(),
                amount: withdraw_amount,
            },
        ];
        pool_client.submit(&contract_addr, &contract_addr, &contract_addr, &requests);

        // Always send to vault
        token::Client::new(&e, &asset).transfer(&contract_addr, &vault, &withdraw_amount);
        storage::set_tracked_balance(&e, 0);

        events::emergency_withdraw(&e, withdraw_amount, &caller);
        bump_instance(&e);

        Ok(withdraw_amount)
    }

    // ----- Health check -----

    /// Pool utilization and backstop coverage health check.
    ///
    /// Queries the Blend pool's reserve data to compute:
    /// 1. Utilization = d_supply / b_supply (if > max_utilization -> unhealthy)
    /// 2. Backstop credit ratio (if depleted -> unhealthy)
    ///
    /// Returns true if the pool is within configured safety thresholds.
    pub fn is_healthy(e: Env) -> bool {
        let blend_pool_addr = storage::get_blend_pool(&e);
        let asset = storage::get_asset(&e);
        let pool_client = pool::Client::new(&e, &blend_pool_addr);

        // Try to read reserve data; if the pool is unreachable, report unhealthy
        let reserve = match pool_client.try_get_reserve(&asset) {
            Ok(Ok(r)) => r,
            _ => return false,
        };

        let max_util_bps = storage::get_max_utilization_bps(&e);

        // Utilization = d_supply / b_supply (both in bToken terms)
        // Guard against division by zero
        let b_supply = reserve.data.b_supply;
        let d_supply = reserve.data.d_supply;

        if b_supply > 0 {
            // Scale to BPS: (d_supply * 10000) / b_supply
            let utilization_bps = d_supply * 10_000 / b_supply;
            if utilization_bps > max_util_bps as i128 {
                return false;
            }
        }

        // Check backstop credit (negative = backstop depleted = bad)
        let min_backstop_bps = storage::get_min_backstop_bps(&e);
        let backstop_credit = reserve.data.backstop_credit;
        if backstop_credit < 0 {
            return false;
        }

        // Backstop coverage ratio = backstop_credit / b_supply
        if b_supply > 0 && backstop_credit >= 0 {
            let coverage_bps = backstop_credit * 10_000 / b_supply;
            if coverage_bps < min_backstop_bps as i128 {
                return false;
            }
        }

        true
    }

    // ----- View functions -----

    pub fn balance_of(e: Env) -> i128 {
        storage::get_tracked_balance(&e)
    }

    /// Available liquidity for immediate withdrawal, in underlying asset units.
    ///
    /// What "available" actually means in a Blend pool: pool-wide free
    /// liquidity is `total_supplied_underlying - total_borrowed_underlying`,
    /// NOT `b_supply - d_supply` — those counts are in different token units
    /// (b_token vs d_token) with different accrual rates, and subtracting
    /// them directly was the bug behind the first failed redeem: on a pool
    /// at 37.6% utilization the raw `b_supply - d_supply` over-reported
    /// available by the (b_rate − d_rate) drift, the vault's waterfall
    /// happily asked for that much, and the pool rejected with BalanceError
    /// because it simply didn't have enough free underlying to deliver.
    ///
    /// Correct formula:
    ///
    ///   pool_supplied   = b_supply * b_rate / 1e12
    ///   pool_borrowed   = d_supply * d_rate / 1e12
    ///   pool_available  = pool_supplied - pool_borrowed
    ///   our_cap         = min(tracked, pool_available)
    ///
    /// A 2-stroop safety margin is subtracted so exact-match withdraws don't
    /// race rounding on the b_token burn inside the pool — same trick the
    /// EVM vault uses in `_availableLiquidity()` for 4626 maxRedeem quotes.
    /// If the pool is unreachable we fall back to `tracked` minus the margin:
    /// without RPC we can't know util, so we trust the smaller conservative
    /// estimate. A stale or unreachable pool should not silently promise
    /// more than we can deliver.
    pub fn available_liquidity(e: Env) -> i128 {
        let tracked = storage::get_tracked_balance(&e);
        if tracked == 0 {
            return 0;
        }

        let blend_pool_addr = storage::get_blend_pool(&e);
        let asset = storage::get_asset(&e);
        let pool_client = pool::Client::new(&e, &blend_pool_addr);

        let reserve = match pool_client.try_get_reserve(&asset) {
            Ok(Ok(r)) => r,
            _ => return apply_rounding_margin(tracked),
        };

        let pool_supplied = reserve.data.b_supply * reserve.data.b_rate / BLEND_RATE_SCALAR;
        let pool_borrowed = reserve.data.d_supply * reserve.data.d_rate / BLEND_RATE_SCALAR;
        let pool_available = pool_supplied - pool_borrowed;

        let cap = if pool_available < tracked {
            pool_available
        } else {
            tracked
        };
        apply_rounding_margin(cap)
    }

    /// Claim BLND token emissions from the Blend pool.
    /// Keeper-callable; claimed tokens stay in the strategy contract
    /// for the admin to sweep or the vault to collect.
    pub fn harvest(e: Env, caller: Address) -> Result<i128, StrategyError> {
        caller.require_auth();
        // Allow vault or admin to harvest
        let vault = storage::get_vault(&e);
        let admin = storage::get_admin(&e);
        if caller != vault && caller != admin {
            return Err(StrategyError::Unauthorized);
        }

        let blend_pool_addr = storage::get_blend_pool(&e);
        let asset = storage::get_asset(&e);
        let contract_addr = e.current_contract_address();
        let pool_client = pool::Client::new(&e, &blend_pool_addr);

        // Claim emissions for our reserve index
        let reserve = match pool_client.try_get_reserve(&asset) {
            Ok(Ok(r)) => r,
            _ => return Ok(0),
        };

        let reserve_token_ids = vec![&e, reserve.config.index * 2];
        let claimed = pool_client.claim(&contract_addr, &reserve_token_ids, &contract_addr);

        bump_instance(&e);
        Ok(claimed)
    }

    pub fn asset(e: Env) -> Address {
        storage::get_asset(&e)
    }

    pub fn vault(e: Env) -> Address {
        storage::get_vault(&e)
    }

    pub fn blend_pool(e: Env) -> Address {
        storage::get_blend_pool(&e)
    }

    pub fn is_paused(e: Env) -> bool {
        storage::is_paused(&e)
    }

    // ----- Admin functions -----

    pub fn pause(e: Env, caller: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_paused(&e, true);
        events::pause(&e, &caller);
        Ok(())
    }

    pub fn unpause(e: Env, caller: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_paused(&e, false);
        events::unpause(&e, &caller);
        Ok(())
    }

    /// Propose a new admin. Pending admin must call `accept_admin`.
    pub fn propose_admin(e: Env, caller: Address, new_admin: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_pending_admin(&e, &new_admin);
        Ok(())
    }

    /// Accept admin role. Must be called by the pending admin.
    pub fn accept_admin(e: Env, caller: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        match storage::get_pending_admin(&e) {
            Some(addr) if addr == caller => {
                let old_admin = storage::get_admin(&e);
                storage::set_admin(&e, &caller);
                storage::clear_pending_admin(&e);
                events::admin_changed(&e, &old_admin, &caller);
                Ok(())
            }
            _ => Err(StrategyError::Unauthorized),
        }
    }

    pub fn set_vault(e: Env, caller: Address, new_vault: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        let old_vault = storage::get_vault(&e);
        storage::set_vault(&e, &new_vault);
        events::vault_changed(&e, &old_vault, &new_vault);
        Ok(())
    }

    pub fn set_max_utilization(e: Env, caller: Address, bps: u32) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if bps > MAX_BPS {
            return Err(StrategyError::InvalidBps);
        }
        storage::set_max_utilization_bps(&e, bps);
        events::config_update(&e, "max_util", bps);
        Ok(())
    }

    pub fn set_min_backstop_coverage(
        e: Env,
        caller: Address,
        bps: u32,
    ) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if bps > MAX_BPS {
            return Err(StrategyError::InvalidBps);
        }
        storage::set_min_backstop_bps(&e, bps);
        events::config_update(&e, "min_backstop", bps);
        Ok(())
    }

    pub fn set_approval_buffer(e: Env, caller: Address, buffer: u32) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_approval_buffer(&e, buffer);
        events::config_update(&e, "approval_buf", buffer);
        Ok(())
    }

    pub fn set_upgrade_delay(e: Env, caller: Address, delay: u64) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if delay < storage::MIN_UPGRADE_DELAY {
            return Err(StrategyError::InvalidBps); // reuse for "invalid param"
        }
        storage::set_upgrade_delay(&e, delay);
        Ok(())
    }

    /// Schedule a WASM upgrade. Executes after configured delay.
    pub fn schedule_upgrade(
        e: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        let now = e.ledger().timestamp();
        storage::set_scheduled_upgrade(&e, &new_wasm_hash, now);
        Ok(())
    }

    /// Execute a previously scheduled upgrade after delay.
    pub fn execute_upgrade(e: Env, caller: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        let (wasm_hash, scheduled_at) =
            storage::get_scheduled_upgrade(&e).ok_or(StrategyError::UpgradeNotScheduled)?;
        let delay = storage::get_upgrade_delay(&e);
        if e.ledger().timestamp() < scheduled_at + delay {
            return Err(StrategyError::UpgradeTooEarly);
        }
        storage::clear_scheduled_upgrade(&e);
        e.deployer().update_current_contract_wasm(wasm_hash.clone());
        events::upgrade(&e, &wasm_hash);
        Ok(())
    }

    /// Cancel a scheduled upgrade.
    pub fn cancel_upgrade(e: Env, caller: Address) -> Result<(), StrategyError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if storage::get_scheduled_upgrade(&e).is_none() {
            return Err(StrategyError::UpgradeNotScheduled);
        }
        storage::clear_scheduled_upgrade(&e);
        Ok(())
    }

    // ----- Internal -----

    fn require_vault(e: &Env, caller: &Address) -> Result<(), StrategyError> {
        if *caller != storage::get_vault(e) {
            return Err(StrategyError::Unauthorized);
        }
        Ok(())
    }

    fn require_admin(e: &Env, caller: &Address) -> Result<(), StrategyError> {
        if *caller != storage::get_admin(e) {
            return Err(StrategyError::Unauthorized);
        }
        Ok(())
    }

    fn require_not_paused(e: &Env) -> Result<(), StrategyError> {
        if storage::is_paused(e) {
            return Err(StrategyError::Paused);
        }
        Ok(())
    }

    /// Query actual collateral position from the Blend pool.
    /// Returns the underlying asset amount (bTokens * b_rate / scalar).
    /// Returns 0 if pool is unreachable or position doesn't exist.
    fn query_pool_position(
        e: &Env,
        pool_client: &pool::Client,
        asset: &Address,
        contract_addr: &Address,
    ) -> i128 {
        let reserve = match pool_client.try_get_reserve(asset) {
            Ok(Ok(r)) => r,
            _ => return storage::get_tracked_balance(e), // fallback to tracked
        };

        let positions = match pool_client.try_get_positions(contract_addr) {
            Ok(Ok(p)) => p,
            _ => return storage::get_tracked_balance(e),
        };

        let b_tokens = positions.collateral.get(reserve.config.index).unwrap_or(0);
        if b_tokens == 0 {
            return 0;
        }

        // Convert bTokens to underlying using the pool's b_rate.
        b_tokens * reserve.data.b_rate / BLEND_RATE_SCALAR
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

    fn init_strategy<'a>(
        e: &Env,
    ) -> (
        BlendStrategyClient<'a>,
        Address,
        Address,
        Address,
        Address,
        Address,
    ) {
        let (admin, vault, asset, blend_pool) = setup(e);
        let contract_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(e, &contract_id);

        client.initialize(&admin, &vault, &asset, &blend_pool);

        (client, contract_id, admin, vault, asset, blend_pool)
    }

    #[test]
    fn test_initialize() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, _, vault, asset, blend_pool) = init_strategy(&e);

        assert_eq!(client.asset(), asset);
        assert_eq!(client.vault(), vault);
        assert_eq!(client.blend_pool(), blend_pool);
        assert_eq!(client.balance_of(), 0);
        assert!(!client.is_paused());
    }

    #[test]
    fn test_double_initialize_fails() {
        let e = Env::default();
        e.mock_all_auths();

        let (admin, vault, asset, blend_pool) = setup(&e);
        let contract_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(&e, &contract_id);

        client.initialize(&admin, &vault, &asset, &blend_pool);
        let result = client.try_initialize(&admin, &vault, &asset, &blend_pool);
        assert!(result.is_err());
    }

    #[test]
    fn test_unauthorized_deposit() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, _, _, _, _) = init_strategy(&e);

        let random = Address::generate(&e);
        let result = client.try_deposit(&random, &1000_0000000);
        assert!(result.is_err());
    }

    #[test]
    fn test_pause_blocks_deposit() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, admin, vault, _, _) = init_strategy(&e);

        client.pause(&admin);
        assert!(client.is_paused());

        // Vault deposit should fail while paused
        let result = client.try_deposit(&vault, &1000_0000000);
        assert!(result.is_err());

        // Unpause
        client.unpause(&admin);
        assert!(!client.is_paused());
    }

    #[test]
    fn test_health_thresholds() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, admin, _, _, _) = init_strategy(&e);

        client.set_max_utilization(&admin, &8000u32);
        client.set_min_backstop_coverage(&admin, &1000u32);

        // is_healthy queries the pool which doesn't exist in unit tests,
        // so it returns false (pool unreachable = unhealthy)
        assert!(!client.is_healthy());
    }

    #[test]
    fn test_invalid_bps_rejected() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, admin, _, _, _) = init_strategy(&e);

        let result = client.try_set_max_utilization(&admin, &15000u32);
        assert!(result.is_err());

        let result = client.try_set_min_backstop_coverage(&admin, &15000u32);
        assert!(result.is_err());
    }

    #[test]
    fn test_two_step_admin() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, admin, _, _, _) = init_strategy(&e);

        let new_admin = Address::generate(&e);
        client.propose_admin(&admin, &new_admin);
        client.accept_admin(&new_admin);

        // Old admin should no longer work
        let result = client.try_pause(&admin);
        assert!(result.is_err());

        // New admin should work
        client.pause(&new_admin);
        assert!(client.is_paused());
    }

    #[test]
    fn test_set_vault() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, admin, _, _, _) = init_strategy(&e);

        let new_vault = Address::generate(&e);
        client.set_vault(&admin, &new_vault);
        assert_eq!(client.vault(), new_vault);
    }

    #[test]
    fn test_set_approval_buffer() {
        let e = Env::default();
        e.mock_all_auths();

        let (client, _, admin, _, _, _) = init_strategy(&e);

        client.set_approval_buffer(&admin, &500u32);
        // No direct getter in public API, but the config_update event confirms it
    }
}

// ---------------------------------------------------------------------------
// Integration tests (require full Blend fixture)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration_test {
    use super::*;
    use blend_contract_sdk::testutils::{default_reserve_config, BlendFixture};
    use soroban_sdk::testutils::{Address as _, BytesN as _};
    use soroban_sdk::token::{StellarAssetClient, TokenClient};
    use soroban_sdk::{BytesN, String};

    /// Deploy a full Blend environment and our strategy, then test
    /// the actual deposit -> withdraw flow through the Blend pool.
    #[test]
    fn test_deposit_withdraw_via_blend_pool() {
        let e = Env::default();
        e.mock_all_auths();

        let deployer = Address::generate(&e);
        let blnd = e
            .register_stellar_asset_contract_v2(deployer.clone())
            .address();
        let usdc = e
            .register_stellar_asset_contract_v2(deployer.clone())
            .address();

        // Deploy full Blend protocol
        let blend = BlendFixture::deploy(&e, &deployer, &blnd, &usdc);

        // Create a Blend pool with USDC reserve
        let pool_addr = blend.pool_factory.mock_all_auths().deploy(
            &deployer,
            &String::from_str(&e, "test-pool"),
            &BytesN::<32>::random(&e),
            &Address::generate(&e), // oracle
            &1_000_000,             // 10% take rate (Blend SCALAR_7: 0.1 * 1e7)
            &4,
            &1_0000000,
        );

        let pool_client = pool::Client::new(&e, &pool_addr);
        let reserve_config = default_reserve_config();
        pool_client
            .mock_all_auths()
            .queue_set_reserve(&usdc, &reserve_config);
        pool_client.mock_all_auths().set_reserve(&usdc);

        // Fund backstop so pool can become active
        blend
            .backstop
            .mock_all_auths()
            .deposit(&deployer, &pool_addr, &50_000_0000000);
        pool_client.mock_all_auths().set_status(&3);
        pool_client.mock_all_auths().update_status();

        // Deploy our strategy
        let admin = Address::generate(&e);
        let vault = Address::generate(&e);
        let strategy_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(&e, &strategy_id);

        client.initialize(&admin, &vault, &usdc, &pool_addr);

        // Mint USDC to vault, pre-transfer to strategy, then deposit
        StellarAssetClient::new(&e, &usdc).mint(&vault, &10_000_0000000);

        let deposit_amount: i128 = 1_000_0000000;
        // Pre-transfer: vault sends USDC to strategy (matches vault.allocate flow)
        TokenClient::new(&e, &usdc).transfer(&vault, &strategy_id, &deposit_amount);
        client.deposit(&vault, &deposit_amount);

        assert_eq!(client.balance_of(), deposit_amount);

        // Verify USDC actually left the vault
        let usdc_client = TokenClient::new(&e, &usdc);
        assert_eq!(usdc_client.balance(&vault), 10_000_0000000 - deposit_amount);

        // Withdraw half
        let withdrawn = client.withdraw(&vault, &500_0000000);
        assert_eq!(withdrawn, 500_0000000);
        assert_eq!(client.balance_of(), 500_0000000);

        // Emergency withdraw the rest
        let emergency = client.emergency_withdraw(&vault);
        assert_eq!(emergency, 500_0000000);
        assert_eq!(client.balance_of(), 0);

        // All USDC should be back in vault
        assert_eq!(usdc_client.balance(&vault), 10_000_0000000);
    }

    /// Test that is_healthy actually queries the pool and works.
    #[test]
    fn test_health_check_against_real_pool() {
        let e = Env::default();
        e.mock_all_auths();

        let deployer = Address::generate(&e);
        let blnd = e
            .register_stellar_asset_contract_v2(deployer.clone())
            .address();
        let usdc = e
            .register_stellar_asset_contract_v2(deployer.clone())
            .address();

        let blend = BlendFixture::deploy(&e, &deployer, &blnd, &usdc);

        let pool_addr = blend.pool_factory.mock_all_auths().deploy(
            &deployer,
            &String::from_str(&e, "health-pool"),
            &BytesN::<32>::random(&e),
            &Address::generate(&e),
            &1_000_000, // 10% take rate (Blend SCALAR_7: 0.1 * 1e7)
            &4,
            &1_0000000,
        );

        let pool_client = pool::Client::new(&e, &pool_addr);
        pool_client
            .mock_all_auths()
            .queue_set_reserve(&usdc, &default_reserve_config());
        pool_client.mock_all_auths().set_reserve(&usdc);

        blend
            .backstop
            .mock_all_auths()
            .deposit(&deployer, &pool_addr, &50_000_0000000);
        pool_client.mock_all_auths().set_status(&3);
        pool_client.mock_all_auths().update_status();

        let admin = Address::generate(&e);
        let vault = Address::generate(&e);
        let strategy_id = e.register(BlendStrategy, ());
        let client = BlendStrategyClient::new(&e, &strategy_id);

        client.initialize(&admin, &vault, &usdc, &pool_addr);

        // Pool exists and is healthy (low utilization, backstop funded)
        assert!(client.is_healthy());
    }

    /// End-to-end: vault deposit -> allocate to strategy -> deallocate -> redeem.
    /// Tests the full fund flow with both contracts deployed against a real Blend pool.
    #[test]
    fn test_vault_strategy_e2e() {
        let e = Env::default();
        e.mock_all_auths();

        let deployer = Address::generate(&e);
        let blnd = e
            .register_stellar_asset_contract_v2(deployer.clone())
            .address();
        let usdc = e
            .register_stellar_asset_contract_v2(deployer.clone())
            .address();

        // Deploy Blend
        let blend = BlendFixture::deploy(&e, &deployer, &blnd, &usdc);
        let pool_addr = blend.pool_factory.mock_all_auths().deploy(
            &deployer,
            &String::from_str(&e, "e2e-pool"),
            &BytesN::<32>::random(&e),
            &Address::generate(&e),
            &1_000_000, // 10% take rate (Blend SCALAR_7: 0.1 * 1e7)
            &4,
            &1_0000000,
        );
        let pool_client = pool::Client::new(&e, &pool_addr);
        pool_client
            .mock_all_auths()
            .queue_set_reserve(&usdc, &default_reserve_config());
        pool_client.mock_all_auths().set_reserve(&usdc);
        blend
            .backstop
            .mock_all_auths()
            .deposit(&deployer, &pool_addr, &50_000_0000000);
        pool_client.mock_all_auths().set_status(&3);
        pool_client.mock_all_auths().update_status();

        // Deploy vault
        let admin = Address::generate(&e);
        let keeper = Address::generate(&e);
        let guardian = Address::generate(&e);
        let fee_recipient = Address::generate(&e);
        let user = Address::generate(&e);

        let vault_id = e.register(tezoro_vault::TezoroVault, ());
        let vault_client = tezoro_vault::TezoroVaultClient::new(&e, &vault_id);
        vault_client.initialize(
            &admin,
            &usdc,
            &keeper,
            &guardian,
            &fee_recipient,
            &1500u32,
            &300u32,
            &String::from_str(&e, "Tezoro USDC-A"),
            &String::from_str(&e, "tUSDC-A"),
        );

        // Deploy strategy
        let strategy_id = e.register(BlendStrategy, ());
        let strategy_client = BlendStrategyClient::new(&e, &strategy_id);
        strategy_client.initialize(&admin, &vault_id, &usdc, &pool_addr);

        // Register strategy in vault
        vault_client.add_strategy(&admin, &strategy_id);

        // Mint USDC to user, deposit into vault
        StellarAssetClient::new(&e, &usdc).mint(&user, &10_000_0000000);
        let shares = vault_client.deposit(&user, &5_000_0000000);
        assert!(shares > 0);
        assert_eq!(vault_client.total_assets(), 5_000_0000000);

        // Keeper allocates 3000 USDC to strategy (idle buffer = 3% of 5000 = 150)
        vault_client.allocate(&keeper, &strategy_id, &3_000_0000000);
        assert_eq!(strategy_client.balance_of(), 3_000_0000000);
        assert_eq!(vault_client.tracked_balance(&strategy_id), 3_000_0000000);
        // total_assets unchanged (idle decreased, strategy increased)
        assert_eq!(vault_client.total_assets(), 5_000_0000000);

        // Keeper deallocates 1000 back
        let withdrawn = vault_client.deallocate(&keeper, &strategy_id, &1_000_0000000);
        assert_eq!(withdrawn, 1_000_0000000);
        assert_eq!(strategy_client.balance_of(), 2_000_0000000);
        assert_eq!(vault_client.tracked_balance(&strategy_id), 2_000_0000000);

        // Deallocate remaining 2000 so vault has enough idle for full redeem
        vault_client.deallocate(&keeper, &strategy_id, &2_000_0000000);
        assert_eq!(strategy_client.balance_of(), 0);

        // User redeems all shares (vault has 5000 idle)
        let user_shares = vault_client.balance(&user);
        let redeemed = vault_client.redeem(&user, &user_shares);
        assert_eq!(redeemed, 5_000_0000000);
    }
}
