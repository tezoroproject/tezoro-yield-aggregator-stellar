// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

/// @title IStrategy - Universal market adapter interface
/// @notice Every lending market adapter implements this interface.
///         Adding a new market = deploying a new contract that implements IStrategy.
///         Zero vault code changes.
interface IStrategy {
    /// @notice Deposit assets into the underlying lending protocol
    /// @param amount Amount of the underlying asset to deposit
    function deposit(uint256 amount) external;

    /// @notice Withdraw assets from the underlying lending protocol
    /// @param amount Amount of the underlying asset to withdraw
    /// @return withdrawn Actual amount withdrawn (may be less than requested if liquidity is insufficient)
    function withdraw(uint256 amount) external returns (uint256 withdrawn);

    /// @notice Emergency withdrawal -- best-effort, returns whatever is available
    /// @return withdrawn Amount actually withdrawn
    function emergencyWithdraw() external returns (uint256 withdrawn);

    /// @notice Current balance of assets deposited in the underlying protocol
    /// @return Total assets controlled by this strategy (principal + accrued yield)
    function balanceOf() external view returns (uint256);

    /// @notice Amount that can be withdrawn right now without waiting
    /// @return Available liquidity in the underlying protocol for this strategy
    function availableLiquidity() external view returns (uint256);

    /// @notice Check if the underlying protocol is in a healthy state
    /// @return True if the protocol pool is healthy and operational
    function isHealthy() external view returns (bool);

    /// @notice The underlying asset this strategy operates on
    /// @return Address of the ERC-20 token
    function asset() external view returns (address);

    /// @notice Harvest any pending rewards from the underlying protocol
    /// @param rewardsModule Address to send harvested reward tokens to
    /// @return Amount of reward tokens harvested (0 if no rewards or rewards module not set)
    function harvest(address rewardsModule) external returns (uint256);

    /// @notice Sweep non-asset reward tokens stuck on the strategy to a recipient.
    ///         Used for merkle-claimed rewards (Morpho URD, Merkl, etc.) that land
    ///         on the strategy address after an external claim.
    /// @param rewardToken The ERC-20 token to sweep (must NOT be the strategy's asset)
    /// @param to Recipient address (typically the RewardsModule)
    /// @return amount Amount of tokens swept
    function sweepReward(address rewardToken, address to) external returns (uint256 amount);
}
