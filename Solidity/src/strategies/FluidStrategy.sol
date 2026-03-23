// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC4626} from "@openzeppelin/contracts/interfaces/IERC4626.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IStrategy} from "../interfaces/IStrategy.sol";

/// @title FluidStrategy
/// @notice Strategy adapter for Fluid lending (fTokens).
///         fTokens are ERC-4626 vaults -- deposit/withdraw via standard interface.
contract FluidStrategy is IStrategy {
    using SafeERC20 for IERC20;

    address public immutable override asset;
    IERC4626 public immutable fToken;
    address public immutable vault;

    error NotVault();
    error ZeroAddress();
    error CannotSweepAsset();

    modifier onlyVault() {
        if (msg.sender != vault) revert NotVault();
        _;
    }

    constructor(address asset_, address fToken_, address vault_) {
        if (asset_ == address(0) || fToken_ == address(0) || vault_ == address(0)) {
            revert ZeroAddress();
        }

        asset = asset_;
        fToken = IERC4626(fToken_);
        vault = vault_;

        // Approve fToken to pull assets for deposit
        IERC20(asset_).forceApprove(fToken_, type(uint256).max);
    }

    function deposit(uint256 amount) external override onlyVault {
        IERC20(asset).safeTransferFrom(vault, address(this), amount);
        fToken.deposit(amount, address(this));
    }

    function withdraw(uint256 amount) external override onlyVault returns (uint256 withdrawn) {
        uint256 available = availableLiquidity();
        uint256 toWithdraw = amount > available ? available : amount;
        if (toWithdraw == 0) return 0;

        // Measure actual amount received by vault to handle ERC-4626 rounding
        uint256 vaultBalBefore = IERC20(asset).balanceOf(vault);
        fToken.withdraw(toWithdraw, vault, address(this));
        withdrawn = IERC20(asset).balanceOf(vault) - vaultBalBefore;
    }

    function emergencyWithdraw() external override onlyVault returns (uint256 withdrawn) {
        uint256 shares = fToken.balanceOf(address(this));
        if (shares == 0) return 0;

        // Redeem all shares, send assets to vault
        withdrawn = fToken.redeem(shares, vault, address(this));
    }

    function balanceOf() external view override returns (uint256) {
        uint256 shares = fToken.balanceOf(address(this));
        if (shares == 0) return 0;
        return fToken.convertToAssets(shares);
    }

    function availableLiquidity() public view override returns (uint256) {
        // Fluid fTokens use a separate Liquidity layer -- assets are NOT held
        // in the fToken contract. Use ERC-4626 maxWithdraw for accurate result.
        return fToken.maxWithdraw(address(this));
    }

    function isHealthy() external view override returns (bool) {
        // Fluid holds assets in a Liquidity layer, not in fToken directly.
        // Check that the fToken has active deposits (totalAssets > 0).
        return fToken.totalAssets() > 0;
    }

    function harvest(address) external pure override returns (uint256) {
        // Fluid rewards handled externally (Merkl, etc.).
        // Claims are done via RewardsModule.executeClaim(), tokens land here,
        // then swept via sweepReward().
        return 0;
    }

    /// @notice Sweep non-asset reward tokens to a recipient (e.g. RewardsModule).
    function sweepReward(address rewardToken, address to) external override onlyVault returns (uint256 amount) {
        if (rewardToken == asset || rewardToken == address(fToken)) revert CannotSweepAsset();
        amount = IERC20(rewardToken).balanceOf(address(this));
        if (amount > 0) {
            IERC20(rewardToken).safeTransfer(to, amount);
        }
    }
}
