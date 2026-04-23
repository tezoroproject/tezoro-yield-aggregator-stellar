use soroban_sdk::{contractclient, Address, Env};

use crate::StrategyError;

/// Strategy interface for Tezoro vault adapters.
///
/// **Fund flow convention**: The vault pre-transfers tokens to the strategy
/// before calling `deposit`. The strategy deploys from its own balance.
/// This avoids cross-contract auth complexity (strategy doesn't need to
/// pull tokens from the vault).
///
/// Withdraw and emergency_withdraw return tokens to the caller (vault).
///
/// **Error handling**: all mutating methods return `Result<T, StrategyError>`
/// so the vault can pattern-match on specific failure modes (paused,
/// unhealthy, insufficient liquidity). The generated `StrategyClient` also
/// exposes `try_*` variants that surface errors without panicking; the
/// vault uses those when it wants to react differently to different errors
/// (e.g. skip-this-tick vs abort-the-txn).
#[allow(dead_code)] // consumed by #[contractclient] macro to generate StrategyClient
#[contractclient(name = "StrategyClient")]
pub trait StrategyInterface {
    /// Deploy underlying asset (already received from vault) into the yield source.
    /// Vault MUST transfer tokens to strategy before calling this.
    /// Only callable by the vault.
    fn deposit(env: Env, caller: Address, amount: i128) -> Result<(), StrategyError>;

    /// Withdraw underlying asset from the strategy back to the caller.
    /// Returns actual amount withdrawn (may be less than requested).
    /// Only callable by the vault.
    fn withdraw(env: Env, caller: Address, amount: i128) -> Result<i128, StrategyError>;

    /// Pull all funds from the strategy back to the vault.
    /// Callable by vault or admin (emergency).
    fn emergency_withdraw(env: Env, caller: Address) -> Result<i128, StrategyError>;

    /// Current balance in underlying asset terms (internal tracking).
    fn balance_of(env: Env) -> i128;

    /// How much of the strategy's `balance_of` can be withdrawn RIGHT NOW.
    /// Implementations clamp to underlying-protocol liquidity (e.g. a Blend
    /// pool at 100% utilization has `balance_of > 0` but `available_liquidity
    /// == 0` until borrowers repay). The vault's redeem waterfall consults
    /// this before calling `withdraw` so it doesn't request more than the
    /// strategy can actually deliver — which would either revert the whole
    /// redeem or force saturating subtraction handling.
    fn available_liquidity(env: Env) -> i128;

    /// Whether the strategy's yield source is healthy.
    fn is_healthy(env: Env) -> bool;

    /// The underlying asset address.
    fn asset(env: Env) -> Address;
}
