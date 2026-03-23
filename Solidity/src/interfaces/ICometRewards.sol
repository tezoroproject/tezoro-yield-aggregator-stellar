// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

/// @title ICometRewards
/// @notice Minimal interface for Compound V3 CometRewards contract.
///         Used by CompoundV3Strategy to claim COMP rewards.
interface ICometRewards {
    /// @notice Claim accrued COMP rewards for a supplier.
    /// @param comet The Comet (market) contract address
    /// @param src The supplier address to claim for (rewards sent to src)
    /// @param shouldAccrue Whether to accrue rewards before claiming
    function claim(address comet, address src, bool shouldAccrue) external;
}
