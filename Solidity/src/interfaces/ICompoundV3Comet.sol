// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

/// @notice Minimal Compound V3 (Comet) interface -- only the functions we use
interface ICompoundV3Comet {
    function supply(address asset, uint256 amount) external;

    function withdraw(address asset, uint256 amount) external;

    function balanceOf(address account) external view returns (uint256);

    function baseToken() external view returns (address);

    function isSupplyPaused() external view returns (bool);

    function isWithdrawPaused() external view returns (bool);

    function totalSupply() external view returns (uint256);
}
