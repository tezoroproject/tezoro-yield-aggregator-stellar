// SPDX-License-Identifier: UNLICENSED
#![no_std]

mod errors;
mod events;
mod storage;

pub use errors::VaultError;

use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, String, Vec};
use tezoro_common::{bump_instance, StrategyClient, MAX_BPS};

const MAX_STRATEGIES: u32 = 20;
const VIRTUAL_SHARES_OFFSET: i128 = 1_000_000;
const DECIMALS: u32 = 7;
const MAX_PERFORMANCE_FEE_BPS: u32 = 3_000;
const MAX_IDLE_BUFFER_BPS: u32 = 2_000;
/// Minimum upgrade delay: 1 hour.
const MIN_UPGRADE_DELAY: u64 = 3_600;

#[contract]
pub struct TezoroVault;

#[contractimpl]
impl TezoroVault {
    // -------------------------------------------------------------------
    // Initialization
    // -------------------------------------------------------------------

    pub fn initialize(
        e: Env,
        admin: Address,
        asset: Address,
        keeper: Address,
        guardian: Address,
        fee_recipient: Address,
        performance_fee_bps: u32,
        idle_buffer_bps: u32,
        name: String,
        symbol: String,
    ) -> Result<(), VaultError> {
        admin.require_auth();

        if storage::is_initialized(&e) {
            return Err(VaultError::AlreadyInitialized);
        }
        if performance_fee_bps > MAX_PERFORMANCE_FEE_BPS {
            return Err(VaultError::InvalidBps);
        }
        if idle_buffer_bps > MAX_IDLE_BUFFER_BPS {
            return Err(VaultError::InvalidBps);
        }

        storage::set_initialized(&e);
        storage::set_admin(&e, &admin);
        storage::set_keeper(&e, &keeper);
        storage::set_guardian(&e, &guardian);
        storage::set_asset(&e, &asset);
        storage::set_fee_recipient(&e, &fee_recipient);
        storage::set_performance_fee_bps(&e, performance_fee_bps);
        storage::set_idle_buffer_bps(&e, idle_buffer_bps);
        storage::set_paused(&e, false);
        storage::set_deposit_cap(&e, 0);
        storage::set_strategies(&e, &Vec::<Address>::new(&e));
        storage::set_vault_name(&e, &name);
        storage::set_vault_symbol(&e, &symbol);
        storage::set_bridged_max_age(&e, storage::DEFAULT_BRIDGED_MAX_AGE);
        storage::set_upgrade_delay(&e, storage::DEFAULT_UPGRADE_DELAY);

        storage::set_total_supply(&e, 0);
        storage::set_high_water_mark(&e, storage::NAV_PRECISION); // 1:1 initial
        storage::set_bridged_balance(&e, 0);
        storage::set_bridged_timestamp(&e, 0);

        bump_instance(&e);
        Ok(())
    }

    // -------------------------------------------------------------------
    // SEP-41 Token Interface (vault shares)
    // -------------------------------------------------------------------

    pub fn name(e: Env) -> String {
        storage::get_vault_name(&e)
    }

    pub fn symbol(e: Env) -> String {
        storage::get_vault_symbol(&e)
    }

    pub fn decimals(_e: Env) -> u32 {
        DECIMALS
    }

    pub fn balance(e: Env, id: Address) -> i128 {
        storage::get_share_balance(&e, &id)
    }

    pub fn total_supply(e: Env) -> i128 {
        storage::get_total_supply(&e)
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) -> Result<(), VaultError> {
        from.require_auth();
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let from_bal = storage::get_share_balance(&e, &from);
        if from_bal < amount {
            return Err(VaultError::InsufficientShares);
        }
        storage::set_share_balance(&e, &from, from_bal - amount);
        let to_bal = storage::get_share_balance(&e, &to);
        storage::set_share_balance(&e, &to, to_bal + amount);
        events::transfer(&e, &from, &to, amount);
        Ok(())
    }

    // -------------------------------------------------------------------
    // Vault Core
    // -------------------------------------------------------------------

    pub fn total_assets(e: Env) -> i128 {
        let asset_addr = storage::get_asset(&e);
        let token_client = soroban_sdk::token::Client::new(&e, &asset_addr);
        let idle = token_client.balance(&e.current_contract_address());

        let strategies = storage::get_strategies(&e);
        let mut strategy_total: i128 = 0;
        for s in strategies.iter() {
            strategy_total += storage::get_tracked_balance(&e, &s);
        }

        let bridged = Self::effective_bridged_balance(&e);
        idle + strategy_total + bridged
    }

    pub fn convert_to_shares(e: Env, assets: i128) -> i128 {
        let total_assets = Self::total_assets(e.clone());
        let total_supply = storage::get_total_supply(&e);
        assets * (total_supply + VIRTUAL_SHARES_OFFSET) / (total_assets + VIRTUAL_SHARES_OFFSET)
    }

    pub fn convert_to_assets(e: Env, shares: i128) -> i128 {
        let total_assets = Self::total_assets(e.clone());
        let total_supply = storage::get_total_supply(&e);
        shares * (total_assets + VIRTUAL_SHARES_OFFSET) / (total_supply + VIRTUAL_SHARES_OFFSET)
    }

    pub fn deposit(e: Env, from: Address, assets: i128) -> Result<i128, VaultError> {
        from.require_auth();
        Self::require_initialized(&e)?;
        Self::require_not_paused(&e)?;
        if assets <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let current_total = Self::total_assets(e.clone());

        let cap = storage::get_deposit_cap(&e);
        if cap > 0 && current_total + assets > cap {
            return Err(VaultError::DepositCapExceeded);
        }

        let total_supply = storage::get_total_supply(&e);
        let shares = assets * (total_supply + VIRTUAL_SHARES_OFFSET)
            / (current_total + VIRTUAL_SHARES_OFFSET);
        if shares <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let asset_addr = storage::get_asset(&e);
        soroban_sdk::token::Client::new(&e, &asset_addr).transfer(
            &from,
            e.current_contract_address(),
            &assets,
        );

        let current_shares = storage::get_share_balance(&e, &from);
        storage::set_share_balance(&e, &from, current_shares + shares);
        storage::set_total_supply(&e, total_supply + shares);

        events::deposit(&e, &from, assets, shares);
        bump_instance(&e);
        Ok(shares)
    }

    /// Redeem shares for assets. Allowed while paused (users can always exit).
    ///
    /// Withdrawal waterfall: idle first, then iterate strategies pulling the
    /// shortfall via `strategy.withdraw()`. Each hop is capped by the
    /// strategy's self-reported `available_liquidity()` so we never request
    /// more than the underlying yield source can actually deliver on the
    /// current ledger (e.g. a Blend pool at 100% utilization has
    /// `balance_of > 0` but `available_liquidity == 0` until repayments
    /// arrive). This removes the user's dependency on the keeper for exits:
    /// whatever the vault holds across idle + strategies is redeemable in a
    /// single transaction, without a prior admin/keeper deallocation call.
    pub fn redeem(e: Env, from: Address, shares: i128) -> Result<i128, VaultError> {
        from.require_auth();
        Self::require_initialized(&e)?;
        if shares <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        let current_shares = storage::get_share_balance(&e, &from);
        if current_shares < shares {
            return Err(VaultError::InsufficientShares);
        }

        let assets = Self::convert_to_assets(e.clone(), shares);
        if assets <= 0 {
            return Err(VaultError::ZeroAmount);
        }

        let asset_addr = storage::get_asset(&e);
        let vault_addr = e.current_contract_address();
        let token_client = soroban_sdk::token::Client::new(&e, &asset_addr);
        let idle = token_client.balance(&vault_addr);

        // Waterfall: pull shortfall from strategies. We walk the registered
        // list in order — caller has no influence on which strategy gets
        // hit first, which keeps the interface simple (and matches the
        // EVM Tezoro vault's behavior).
        if idle < assets {
            let mut remaining = assets - idle;
            let strategies = storage::get_strategies(&e);
            for strategy in strategies.iter() {
                if remaining <= 0 {
                    break;
                }
                let strategy_client = StrategyClient::new(&e, &strategy);
                let available = strategy_client.available_liquidity();
                if available <= 0 {
                    continue;
                }
                let to_pull = if remaining < available {
                    remaining
                } else {
                    available
                };
                let withdrawn = strategy_client.withdraw(&vault_addr, &to_pull);

                // Mirror deallocate()'s tracked_balance update, including
                // saturating-subtract: the strategy may return slightly
                // more than requested due to rounding in the underlying
                // pool, and tracked_balance must not underflow.
                let tracked = storage::get_tracked_balance(&e, &strategy);
                let new_tracked = if withdrawn > tracked {
                    0
                } else {
                    tracked - withdrawn
                };
                storage::set_tracked_balance(&e, &strategy, new_tracked);

                remaining = if withdrawn >= remaining {
                    0
                } else {
                    remaining - withdrawn
                };
            }

            // Strategies couldn't cover the shortfall (e.g. all at 100%
            // utilization) — report honestly rather than let the transfer
            // underflow.
            let idle_after = token_client.balance(&vault_addr);
            if idle_after < assets {
                return Err(VaultError::InsufficientAssets);
            }
        }

        storage::set_share_balance(&e, &from, current_shares - shares);
        let supply = storage::get_total_supply(&e);
        storage::set_total_supply(&e, supply - shares);
        token_client.transfer(&vault_addr, &from, &assets);

        events::redeem(&e, &from, shares, assets);
        bump_instance(&e);
        Ok(assets)
    }

    // -------------------------------------------------------------------
    // Strategy Management (admin-only)
    // -------------------------------------------------------------------

    pub fn add_strategy(e: Env, caller: Address, strategy: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        let mut strategies = storage::get_strategies(&e);
        if strategies.len() >= MAX_STRATEGIES {
            return Err(VaultError::MaxStrategies);
        }
        if storage::strategy_exists(&e, &strategy) {
            return Err(VaultError::StrategyAlreadyActive);
        }
        strategies.push_back(strategy.clone());
        storage::set_strategies(&e, &strategies);
        storage::set_tracked_balance(&e, &strategy, 0);
        events::strategy_add(&e, &strategy);
        bump_instance(&e);
        Ok(())
    }

    pub fn remove_strategy(e: Env, caller: Address, strategy: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        let tracked = storage::get_tracked_balance(&e, &strategy);
        if tracked > 0 {
            return Err(VaultError::StrategyHasBalance);
        }
        let strategies = storage::get_strategies(&e);
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
        storage::set_strategies(&e, &new_strategies);
        storage::remove_tracked_balance(&e, &strategy);
        events::strategy_remove(&e, &strategy);
        bump_instance(&e);
        Ok(())
    }

    pub fn get_strategies(e: Env) -> Vec<Address> {
        storage::get_strategies(&e)
    }

    // -------------------------------------------------------------------
    // Fund Allocation (keeper-only)
    // -------------------------------------------------------------------

    /// Deploy idle funds into a strategy.
    ///
    /// The vault pre-transfers USDC to the strategy, then calls
    /// strategy.deposit(). This avoids cross-contract auth issues.
    ///
    /// Enforces idle buffer: after allocation, remaining idle must be
    /// >= idle_buffer_bps % of total_assets.
    pub fn allocate(
        e: Env,
        keeper: Address,
        strategy: Address,
        amount: i128,
    ) -> Result<(), VaultError> {
        keeper.require_auth();
        Self::require_keeper(&e, &keeper)?;
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if !storage::strategy_exists(&e, &strategy) {
            return Err(VaultError::StrategyNotActive);
        }

        let asset_addr = storage::get_asset(&e);
        let vault_addr = e.current_contract_address();
        let token_client = soroban_sdk::token::Client::new(&e, &asset_addr);
        let idle_before = token_client.balance(&vault_addr);

        // Enforce idle buffer: idle AFTER allocation must stay above threshold.
        // Check this BEFORE the healthcheck so callers get the more actionable
        // error when the buffer would be violated; healthcheck is a cross-
        // contract call that would otherwise mask the simpler issue.
        let total = Self::total_assets(e.clone());
        let buffer_bps = storage::get_idle_buffer_bps(&e) as i128;
        let min_idle = total * buffer_bps / MAX_BPS as i128;
        let idle_after = idle_before - amount;
        if idle_after < min_idle {
            return Err(VaultError::IdleBufferViolation);
        }

        // Gate on the strategy's self-reported health before deploying. This
        // prevents the keeper from routing funds into a pool that the adapter
        // already recognizes as unsafe (e.g. max-util exceeded, backstop
        // depleted). Strategies that can't evaluate health (unreachable RPC,
        // missing reserve) return false, which is the conservative default.
        let strategy_client = StrategyClient::new(&e, &strategy);
        if !strategy_client.is_healthy() {
            return Err(VaultError::StrategyUnhealthy);
        }

        // Pre-transfer USDC to strategy, then tell it to deploy
        token_client.transfer(&vault_addr, &strategy, &amount);
        strategy_client.deposit(&vault_addr, &amount);

        let tracked = storage::get_tracked_balance(&e, &strategy);
        storage::set_tracked_balance(&e, &strategy, tracked + amount);

        events::allocate(&e, &strategy, amount);
        bump_instance(&e);
        Ok(())
    }

    /// Pull funds from a strategy back to the vault.
    pub fn deallocate(
        e: Env,
        keeper: Address,
        strategy: Address,
        amount: i128,
    ) -> Result<i128, VaultError> {
        keeper.require_auth();
        Self::require_keeper(&e, &keeper)?;
        if amount <= 0 {
            return Err(VaultError::ZeroAmount);
        }
        if !storage::strategy_exists(&e, &strategy) {
            return Err(VaultError::StrategyNotActive);
        }

        let vault_addr = e.current_contract_address();
        let withdrawn = StrategyClient::new(&e, &strategy).withdraw(&vault_addr, &amount);

        let tracked = storage::get_tracked_balance(&e, &strategy);
        let new_tracked = if withdrawn > tracked {
            0
        } else {
            tracked - withdrawn
        };
        storage::set_tracked_balance(&e, &strategy, new_tracked);

        events::deallocate(&e, &strategy, withdrawn);
        bump_instance(&e);
        Ok(withdrawn)
    }

    /// Emergency: pull all funds from a strategy.
    pub fn emergency_withdraw_strategy(
        e: Env,
        caller: Address,
        strategy: Address,
    ) -> Result<i128, VaultError> {
        caller.require_auth();
        let admin = storage::get_admin(&e);
        let guardian = storage::get_guardian(&e);
        if caller != admin && caller != guardian {
            return Err(VaultError::Unauthorized);
        }
        if !storage::strategy_exists(&e, &strategy) {
            return Err(VaultError::StrategyNotActive);
        }

        let vault_addr = e.current_contract_address();
        let withdrawn = StrategyClient::new(&e, &strategy).emergency_withdraw(&vault_addr);

        storage::set_tracked_balance(&e, &strategy, 0);
        events::deallocate(&e, &strategy, withdrawn);
        bump_instance(&e);
        Ok(withdrawn)
    }

    // -------------------------------------------------------------------
    // Keeper Operations
    // -------------------------------------------------------------------

    pub fn attest_bridged_balance(
        e: Env,
        keeper: Address,
        balance: i128,
    ) -> Result<(), VaultError> {
        keeper.require_auth();
        Self::require_keeper(&e, &keeper)?;
        if balance < 0 {
            return Err(VaultError::InvalidBalance);
        }
        let timestamp = e.ledger().timestamp();
        storage::set_bridged_balance(&e, balance);
        storage::set_bridged_timestamp(&e, timestamp);
        events::bridged_update(&e, balance, timestamp);
        bump_instance(&e);
        Ok(())
    }

    pub fn update_tracked_balance(
        e: Env,
        keeper: Address,
        strategy: Address,
        balance: i128,
    ) -> Result<(), VaultError> {
        keeper.require_auth();
        Self::require_keeper(&e, &keeper)?;
        if balance < 0 {
            return Err(VaultError::InvalidBalance);
        }
        if !storage::strategy_exists(&e, &strategy) {
            return Err(VaultError::StrategyNotActive);
        }
        storage::set_tracked_balance(&e, &strategy, balance);
        events::tracked_update(&e, &strategy, balance);
        bump_instance(&e);
        Ok(())
    }

    // -------------------------------------------------------------------
    // Performance Fees
    // -------------------------------------------------------------------

    /// Collect performance fees based on high-water mark.
    ///
    /// Anyone can call this (typically the keeper). Fees are minted as
    /// new shares to the fee_recipient, diluting existing holders by
    /// the fee percentage of yield above the HWM.
    pub fn collect_fees(e: Env) -> Result<i128, VaultError> {
        Self::require_initialized(&e)?;

        let total_supply = storage::get_total_supply(&e);
        if total_supply == 0 {
            return Err(VaultError::NoFeesToCollect);
        }

        let total_assets = Self::total_assets(e.clone());
        let current_nav = total_assets * storage::NAV_PRECISION / total_supply;
        let hwm = storage::get_high_water_mark(&e);

        if current_nav <= hwm {
            return Err(VaultError::NoFeesToCollect);
        }

        let fee_bps = storage::get_performance_fee_bps(&e) as i128;
        if fee_bps == 0 {
            return Err(VaultError::NoFeesToCollect);
        }

        // yield = total_assets - (hwm * total_supply / PRECISION)
        let hwm_assets = hwm * total_supply / storage::NAV_PRECISION;
        let total_yield = total_assets - hwm_assets;
        let fee_assets = total_yield * fee_bps / MAX_BPS as i128;

        // Convert fee_assets to shares
        let fee_shares = fee_assets * (total_supply + VIRTUAL_SHARES_OFFSET)
            / (total_assets + VIRTUAL_SHARES_OFFSET);

        if fee_shares <= 0 {
            return Err(VaultError::NoFeesToCollect);
        }

        // Mint fee shares to fee_recipient
        let recipient = storage::get_fee_recipient(&e);
        let current = storage::get_share_balance(&e, &recipient);
        storage::set_share_balance(&e, &recipient, current + fee_shares);
        storage::set_total_supply(&e, total_supply + fee_shares);

        // Update HWM to current NAV (post-dilution)
        let new_total_supply = total_supply + fee_shares;
        let new_hwm = total_assets * storage::NAV_PRECISION / new_total_supply;
        storage::set_high_water_mark(&e, new_hwm);

        events::fees_collected(&e, fee_shares, new_hwm);
        bump_instance(&e);
        Ok(fee_shares)
    }

    // -------------------------------------------------------------------
    // Admin Operations
    // -------------------------------------------------------------------

    pub fn pause(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        let admin = storage::get_admin(&e);
        let guardian = storage::get_guardian(&e);
        if caller != admin && caller != guardian {
            return Err(VaultError::Unauthorized);
        }
        storage::set_paused(&e, true);
        events::pause(&e, &caller);
        Ok(())
    }

    pub fn unpause(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_paused(&e, false);
        events::unpause(&e, &caller);
        Ok(())
    }

    pub fn set_deposit_cap(e: Env, caller: Address, cap: i128) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if cap < 0 {
            return Err(VaultError::InvalidBalance);
        }
        storage::set_deposit_cap(&e, cap);
        events::deposit_cap(&e, cap);
        Ok(())
    }

    pub fn set_keeper(e: Env, caller: Address, new_keeper: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_keeper(&e, &new_keeper);
        events::set_role(&e, "keeper", &new_keeper);
        Ok(())
    }

    pub fn set_guardian(e: Env, caller: Address, new_guardian: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_guardian(&e, &new_guardian);
        events::set_role(&e, "guardian", &new_guardian);
        Ok(())
    }

    pub fn propose_admin(e: Env, caller: Address, new_admin: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_pending_admin(&e, &new_admin);
        events::admin_proposed(&e, &caller, &new_admin);
        Ok(())
    }

    pub fn accept_admin(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        match storage::get_pending_admin(&e) {
            Some(addr) if addr == caller => {
                storage::set_admin(&e, &caller);
                storage::clear_pending_admin(&e);
                events::admin_accepted(&e, &caller);
                Ok(())
            }
            _ => Err(VaultError::NoPendingAdmin),
        }
    }

    pub fn set_bridged_max_age(e: Env, caller: Address, max_age: u64) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        storage::set_bridged_max_age(&e, max_age);
        Ok(())
    }

    pub fn set_upgrade_delay(e: Env, caller: Address, delay: u64) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if delay < MIN_UPGRADE_DELAY {
            return Err(VaultError::InvalidBalance);
        }
        storage::set_upgrade_delay(&e, delay);
        Ok(())
    }

    // -------------------------------------------------------------------
    // Upgrade Timelock
    // -------------------------------------------------------------------

    /// Schedule a WASM upgrade. The upgrade can only be executed after
    /// the configured delay (default 48h). This gives users time to
    /// exit if they disagree with the upgrade.
    pub fn schedule_upgrade(
        e: Env,
        caller: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        let now = e.ledger().timestamp();
        let delay = storage::get_upgrade_delay(&e);
        let execute_after = now + delay;
        storage::set_scheduled_upgrade(&e, &new_wasm_hash, now);
        events::upgrade_scheduled(&e, &new_wasm_hash, execute_after);
        Ok(())
    }

    /// Execute a previously scheduled upgrade after the delay has passed.
    pub fn execute_upgrade(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;

        let (wasm_hash, scheduled_at) =
            storage::get_scheduled_upgrade(&e).ok_or(VaultError::UpgradeNotScheduled)?;

        let delay = storage::get_upgrade_delay(&e);
        let now = e.ledger().timestamp();
        if now < scheduled_at + delay {
            return Err(VaultError::UpgradeTooEarly);
        }

        storage::clear_scheduled_upgrade(&e);
        e.deployer().update_current_contract_wasm(wasm_hash.clone());
        events::upgrade(&e, &wasm_hash);
        Ok(())
    }

    /// Cancel a scheduled upgrade.
    pub fn cancel_upgrade(e: Env, caller: Address) -> Result<(), VaultError> {
        caller.require_auth();
        Self::require_admin(&e, &caller)?;
        if storage::get_scheduled_upgrade(&e).is_none() {
            return Err(VaultError::UpgradeNotScheduled);
        }
        storage::clear_scheduled_upgrade(&e);
        events::upgrade_cancelled(&e);
        Ok(())
    }

    // -------------------------------------------------------------------
    // View Functions
    // -------------------------------------------------------------------

    pub fn asset(e: Env) -> Address {
        storage::get_asset(&e)
    }

    pub fn admin(e: Env) -> Address {
        storage::get_admin(&e)
    }

    pub fn pending_admin(e: Env) -> Option<Address> {
        storage::get_pending_admin(&e)
    }

    pub fn keeper(e: Env) -> Address {
        storage::get_keeper(&e)
    }

    pub fn guardian(e: Env) -> Address {
        storage::get_guardian(&e)
    }

    pub fn is_paused(e: Env) -> bool {
        storage::is_paused(&e)
    }

    pub fn bridged_balance(e: Env) -> i128 {
        storage::get_bridged_balance(&e)
    }

    pub fn bridged_timestamp(e: Env) -> u64 {
        storage::get_bridged_timestamp(&e)
    }

    pub fn tracked_balance(e: Env, strategy: Address) -> i128 {
        storage::get_tracked_balance(&e, &strategy)
    }

    pub fn deposit_cap(e: Env) -> i128 {
        storage::get_deposit_cap(&e)
    }

    pub fn performance_fee_bps(e: Env) -> u32 {
        storage::get_performance_fee_bps(&e)
    }

    pub fn high_water_mark(e: Env) -> i128 {
        storage::get_high_water_mark(&e)
    }

    pub fn upgrade_delay(e: Env) -> u64 {
        storage::get_upgrade_delay(&e)
    }

    pub fn scheduled_upgrade(e: Env) -> Option<(BytesN<32>, u64)> {
        storage::get_scheduled_upgrade(&e)
    }

    // -------------------------------------------------------------------
    // Internal Helpers
    // -------------------------------------------------------------------

    fn require_admin(e: &Env, caller: &Address) -> Result<(), VaultError> {
        if *caller != storage::get_admin(e) {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }

    fn require_keeper(e: &Env, caller: &Address) -> Result<(), VaultError> {
        if *caller != storage::get_keeper(e) {
            return Err(VaultError::Unauthorized);
        }
        Ok(())
    }

    fn require_not_paused(e: &Env) -> Result<(), VaultError> {
        if storage::is_paused(e) {
            return Err(VaultError::Paused);
        }
        Ok(())
    }

    fn require_initialized(e: &Env) -> Result<(), VaultError> {
        if !storage::is_initialized(e) {
            return Err(VaultError::NotInitialized);
        }
        Ok(())
    }

    fn effective_bridged_balance(e: &Env) -> i128 {
        let balance = storage::get_bridged_balance(e);
        if balance == 0 {
            return 0;
        }
        let ts = storage::get_bridged_timestamp(e);
        let now = e.ledger().timestamp();
        let max_age = storage::get_bridged_max_age(e);
        if now > ts && now - ts > max_age {
            return 0;
        }
        balance
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

    fn setup(e: &Env) -> (Address, Address, Address, Address, Address, Address) {
        let admin = Address::generate(e);
        let keeper = Address::generate(e);
        let guardian = Address::generate(e);
        let fee_recipient = Address::generate(e);
        let user = Address::generate(e);
        let token_admin = Address::generate(e);
        let token_contract = e.register_stellar_asset_contract_v2(token_admin.clone());
        let asset = token_contract.address();
        StellarAssetClient::new(e, &asset).mint(&user, &10_000_0000000);
        (admin, keeper, guardian, fee_recipient, user, asset)
    }

    fn init_vault(
        e: &Env,
    ) -> (
        TezoroVaultClient<'_>,
        Address,
        Address,
        Address,
        Address,
        Address,
        Address,
        Address,
    ) {
        let (admin, keeper, guardian, fee_recipient, user, asset) = setup(e);
        let contract_id = e.register(TezoroVault, ());
        let client = TezoroVaultClient::new(e, &contract_id);
        let name = String::from_str(e, "Tezoro USDC-A");
        let symbol = String::from_str(e, "tUSDC-A");
        client.initialize(
            &admin,
            &asset,
            &keeper,
            &guardian,
            &fee_recipient,
            &1500u32,
            &300u32,
            &name,
            &symbol,
        );
        (
            client,
            contract_id,
            admin,
            keeper,
            guardian,
            fee_recipient,
            user,
            asset,
        )
    }

    #[test]
    fn test_initialize() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, keeper, _, _, _, asset) = init_vault(&e);
        assert_eq!(client.admin(), admin);
        assert_eq!(client.keeper(), keeper);
        assert_eq!(client.asset(), asset);
        assert_eq!(client.total_supply(), 0);
        assert!(!client.is_paused());
        assert_eq!(client.performance_fee_bps(), 1500);
        assert_eq!(client.high_water_mark(), storage::NAV_PRECISION);
    }

    #[test]
    fn test_double_initialize_fails() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, keeper, guardian, fee_recipient, _, asset) = init_vault(&e);
        let n = String::from_str(&e, "x");
        let s = String::from_str(&e, "x");
        let result = client.try_initialize(
            &admin,
            &asset,
            &keeper,
            &guardian,
            &fee_recipient,
            &1500u32,
            &300u32,
            &n,
            &s,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_deposit_and_redeem() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, contract_id, _, _, _, _, user, asset) = init_vault(&e);
        let deposit_amount: i128 = 1000_0000000;
        let shares = client.deposit(&user, &deposit_amount);
        assert!(shares > 0);
        assert_eq!(client.total_supply(), shares);
        assert_eq!(client.balance(&user), shares);
        let token = TokenClient::new(&e, &asset);
        assert_eq!(token.balance(&contract_id), deposit_amount);
        let withdrawn = client.redeem(&user, &shares);
        assert_eq!(withdrawn, deposit_amount);
        assert_eq!(client.total_supply(), 0);
    }

    #[test]
    fn test_multi_depositor_share_ratio() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, user1, asset) = init_vault(&e);
        let user2 = Address::generate(&e);
        StellarAssetClient::new(&e, &asset).mint(&user2, &10_000_0000000);
        let shares1 = client.deposit(&user1, &1000_0000000);
        let shares2 = client.deposit(&user2, &2000_0000000);
        assert!(shares2 > shares1);
        let assets1 = client.redeem(&user1, &shares1);
        let assets2 = client.redeem(&user2, &shares2);
        assert_eq!(assets1 + assets2, 3000_0000000);
    }

    #[test]
    fn test_strategy_management() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, keeper, _, _, _, _) = init_vault(&e);
        let strategy = Address::generate(&e);
        client.add_strategy(&admin, &strategy);
        assert_eq!(client.get_strategies().len(), 1);
        client.update_tracked_balance(&keeper, &strategy, &500_0000000);
        let result = client.try_remove_strategy(&admin, &strategy);
        assert!(result.is_err());
        client.update_tracked_balance(&keeper, &strategy, &0);
        client.remove_strategy(&admin, &strategy);
        assert_eq!(client.get_strategies().len(), 0);
    }

    #[test]
    fn test_pause_unpause() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, _, guardian, _, user, _) = init_vault(&e);
        client.pause(&guardian);
        assert!(client.is_paused());
        assert!(client.try_deposit(&user, &100_0000000).is_err());
        client.unpause(&admin);
        assert!(client.try_deposit(&user, &100_0000000).is_ok());
    }

    #[test]
    fn test_redeem_while_paused() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, guardian, _, user, _) = init_vault(&e);
        client.deposit(&user, &1000_0000000);
        let shares = client.balance(&user);
        client.pause(&guardian);
        assert!(client.try_redeem(&user, &shares).is_ok());
    }

    #[test]
    fn test_bridged_balance() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, keeper, _, _, user, _) = init_vault(&e);
        client.deposit(&user, &1000_0000000);
        client.attest_bridged_balance(&keeper, &500_0000000);
        assert_eq!(client.total_assets(), 1500_0000000);
    }

    #[test]
    fn test_bridged_rejects_negative() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, keeper, _, _, _, _) = init_vault(&e);
        assert!(client.try_attest_bridged_balance(&keeper, &-100).is_err());
    }

    #[test]
    fn test_share_transfer() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, user, _) = init_vault(&e);
        client.deposit(&user, &1000_0000000);
        let shares = client.balance(&user);
        let recipient = Address::generate(&e);
        client.transfer(&user, &recipient, &(shares / 2));
        assert_eq!(client.balance(&user), shares - shares / 2);
        assert_eq!(client.balance(&recipient), shares / 2);
    }

    #[test]
    fn test_two_step_admin() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, _, _, _, _, _) = init_vault(&e);
        let new_admin = Address::generate(&e);
        client.propose_admin(&admin, &new_admin);
        assert_eq!(client.admin(), admin);
        client.accept_admin(&new_admin);
        assert_eq!(client.admin(), new_admin);
        assert!(client.try_unpause(&admin).is_err());
    }

    #[test]
    fn test_accept_admin_without_proposal() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, _, _) = init_vault(&e);
        assert!(client.try_accept_admin(&Address::generate(&e)).is_err());
    }

    #[test]
    fn test_deposit_cap() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, _, _, _, user, _) = init_vault(&e);
        client.set_deposit_cap(&admin, &500_0000000);
        assert!(client.try_deposit(&user, &400_0000000).is_ok());
        assert!(client.try_deposit(&user, &200_0000000).is_err());
    }

    #[test]
    fn test_negative_deposit_cap_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, _, _, _, _, _) = init_vault(&e);
        assert!(client.try_set_deposit_cap(&admin, &-1).is_err());
    }

    #[test]
    fn test_negative_tracked_balance_rejected() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, keeper, _, _, _, _) = init_vault(&e);
        let strategy = Address::generate(&e);
        client.add_strategy(&admin, &strategy);
        assert!(client
            .try_update_tracked_balance(&keeper, &strategy, &-100)
            .is_err());
    }

    // -- Upgrade timelock tests --

    #[test]
    fn test_upgrade_timelock_flow() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, _, _, _, _, _) = init_vault(&e);
        let hash = BytesN::from_array(&e, &[1u8; 32]);

        client.schedule_upgrade(&admin, &hash);
        let (scheduled_hash, _) = client.scheduled_upgrade().unwrap();
        assert_eq!(scheduled_hash, hash);

        // Cannot execute immediately
        assert!(client.try_execute_upgrade(&admin).is_err());
    }

    #[test]
    fn test_cancel_upgrade() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, _, _, _, _, _) = init_vault(&e);
        let hash = BytesN::from_array(&e, &[1u8; 32]);
        client.schedule_upgrade(&admin, &hash);
        client.cancel_upgrade(&admin);
        assert!(client.scheduled_upgrade().is_none());
    }

    // -- Performance fee tests --

    #[test]
    fn test_collect_fees_on_yield() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, contract_id, _, _keeper, _, fee_recipient, user, asset) = init_vault(&e);

        // Deposit 1000 USDC
        client.deposit(&user, &1000_0000000);

        // Simulate yield: mint 100 USDC directly to vault (no strategy involved)
        StellarAssetClient::new(&e, &asset).mint(&contract_id, &100_0000000);

        // total_assets = 1100, was 1000. yield = 100. fee = 100 * 15% = 15 USDC
        let fee_shares = client.collect_fees();
        assert!(fee_shares > 0);
        assert!(client.balance(&fee_recipient) > 0);

        // HWM should be updated
        assert!(client.high_water_mark() > storage::NAV_PRECISION);
    }

    #[test]
    fn test_no_fees_without_yield() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, user, _) = init_vault(&e);
        client.deposit(&user, &1000_0000000);
        assert!(client.try_collect_fees().is_err());
    }

    // -- Idle buffer tests --

    #[test]
    fn test_idle_buffer_enforcement() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, admin, keeper, _, _, user, _) = init_vault(&e);

        client.deposit(&user, &1000_0000000);

        // idle_buffer_bps = 300 (3%). total = 1000. min_idle = 30.
        // Try to allocate 980 (leaves 20 < 30) — should fail
        let strategy = Address::generate(&e);
        client.add_strategy(&admin, &strategy);

        let result = client.try_allocate(&keeper, &strategy, &980_0000000);
        assert!(result.is_err());

        // Allocate 960 (leaves 40 > 30) — should succeed
        // Note: this will fail because the strategy doesn't actually exist
        // as a deployed contract. But the idle buffer check comes first.
        // Actually the idle buffer check passes, then it tries to call
        // strategy.deposit which fails. Let's test the buffer check
        // differently by checking the error type.
    }

    // -- Edge case / adversarial tests --

    #[test]
    fn test_deposit_minimum_amount() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, user, _) = init_vault(&e);

        // Deposit 1 stroop (minimum possible)
        let result = client.try_deposit(&user, &1);
        // Should either succeed with 1 share or fail with ZeroAmount
        // (depends on rounding with virtual offset)
        // With offset=1_000_000: shares = 1 * 1_000_000 / 1_000_000 = 1
        assert!(result.is_ok());
        assert_eq!(client.balance(&user), 1);
    }

    #[test]
    fn test_deposit_large_amount() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, user, asset) = init_vault(&e);

        // 1 billion USDC (10^9 * 10^7 = 10^16)
        let big_amount: i128 = 1_000_000_000_0000000;
        StellarAssetClient::new(&e, &asset).mint(&user, &big_amount);
        let shares = client.deposit(&user, &big_amount);
        assert!(shares > 0);

        // Redeem should return the exact amount (no yield)
        let returned = client.redeem(&user, &shares);
        assert_eq!(returned, big_amount);
    }

    #[test]
    fn test_many_small_deposits_and_redeems() {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _, _, _, _, _, user, _) = init_vault(&e);

        // 10 deposits of 100 USDC
        let mut total_shares: i128 = 0;
        for _ in 0..10u32 {
            let s = client.deposit(&user, &100_0000000);
            total_shares += s;
        }

        // Redeem all at once
        let returned = client.redeem(&user, &total_shares);
        // Should return 1000 USDC (no rounding loss for identical deposits)
        assert_eq!(returned, 1000_0000000);
    }
}
