// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

/// @title ITezoroV1_1
/// @notice Minimal interface for RewardsModule to interact with the vault.
interface ITezoroV1_1 {
    function asset() external view returns (address);
    function depositRewards(uint256 amount) external;
}
