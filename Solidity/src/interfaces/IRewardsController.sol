// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

/// @title IRewardsController
/// @notice Minimal interface for Aave V3 / Spark incentives controller.
///         Used by AaveV3Strategy to claim reward tokens (ARB, OP, etc.).
interface IRewardsController {
    /// @notice Claims all accrued rewards for the given assets and sends to `to`.
    /// @param assets Array of aToken/spToken addresses to claim rewards for
    /// @param to Address to receive the reward tokens
    /// @return rewardsList Addresses of the reward tokens claimed
    /// @return claimedAmounts Amounts claimed per reward token
    function claimAllRewards(address[] calldata assets, address to)
        external
        returns (address[] memory rewardsList, uint256[] memory claimedAmounts);

    /// @notice Returns the list of reward token addresses for a given asset.
    /// @param asset The aToken/spToken address
    /// @return Array of reward token addresses (empty if no active rewards)
    function getRewardsByAsset(address asset) external view returns (address[] memory);
}
