// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IStrategy} from "../interfaces/IStrategy.sol";
import {ICompoundV3Comet} from "../interfaces/ICompoundV3Comet.sol";
import {ICometRewards} from "../interfaces/ICometRewards.sol";

/// @title CompoundV3Strategy
/// @notice Strategy adapter for Compound V3 (Comet).
///         Supplies and withdraws a single base asset.
///         Harvests COMP rewards via CometRewards contract.
contract CompoundV3Strategy is IStrategy {
    using SafeERC20 for IERC20;

    address public immutable override asset;
    ICompoundV3Comet public immutable comet;
    address public immutable vault;
    ICometRewards public immutable cometRewards;
    IERC20 public immutable rewardToken;

    error NotVault();
    error ZeroAddress();
    error CannotSweepAsset();

    modifier onlyVault() {
        if (msg.sender != vault) revert NotVault();
        _;
    }

    /// @param cometRewards_ CometRewards contract. address(0) = no rewards.
    /// @param rewardToken_ COMP token address on this chain. address(0) = no rewards.
    constructor(address asset_, address comet_, address vault_, address cometRewards_, address rewardToken_) {
        if (asset_ == address(0) || comet_ == address(0) || vault_ == address(0)) {
            revert ZeroAddress();
        }

        asset = asset_;
        comet = ICompoundV3Comet(comet_);
        vault = vault_;
        cometRewards = ICometRewards(cometRewards_);
        rewardToken = IERC20(rewardToken_);

        // Approve Comet to pull assets for supply
        IERC20(asset_).forceApprove(comet_, type(uint256).max);
    }

    function deposit(uint256 amount) external override onlyVault {
        IERC20(asset).safeTransferFrom(vault, address(this), amount);
        comet.supply(asset, amount);
    }

    function withdraw(uint256 amount) external override onlyVault returns (uint256 withdrawn) {
        uint256 available = availableLiquidity();
        uint256 toWithdraw = amount > available ? available : amount;
        if (toWithdraw == 0) return 0;

        uint256 balanceBefore = IERC20(asset).balanceOf(address(this));
        comet.withdraw(asset, toWithdraw);
        withdrawn = IERC20(asset).balanceOf(address(this)) - balanceBefore;

        IERC20(asset).safeTransfer(vault, withdrawn);
    }

    function emergencyWithdraw() external override onlyVault returns (uint256 withdrawn) {
        uint256 balance = comet.balanceOf(address(this));
        if (balance == 0) return 0;

        // Cap at pool liquidity — avoids revert when Compound is at 100% utilization
        uint256 liquid = availableLiquidity();
        uint256 toWithdraw = balance > liquid ? liquid : balance;
        if (toWithdraw == 0) return 0;

        uint256 balanceBefore = IERC20(asset).balanceOf(address(this));
        comet.withdraw(asset, toWithdraw);
        withdrawn = IERC20(asset).balanceOf(address(this)) - balanceBefore;

        if (withdrawn > 0) {
            IERC20(asset).safeTransfer(vault, withdrawn);
        }
    }

    function balanceOf() external view override returns (uint256) {
        return comet.balanceOf(address(this));
    }

    function availableLiquidity() public view override returns (uint256) {
        uint256 ourBalance = comet.balanceOf(address(this));
        // Comet holds the base token directly -- check pool liquidity
        uint256 poolLiquidity = IERC20(asset).balanceOf(address(comet));
        return ourBalance > poolLiquidity ? poolLiquidity : ourBalance;
    }

    function isHealthy() external view override returns (bool) {
        // If supply or withdraw is paused, consider unhealthy
        return !comet.isSupplyPaused() && !comet.isWithdrawPaused();
    }

    /// @notice Claim COMP rewards and forward to rewardsModule.
    ///         CometRewards sends COMP to this contract, then we transfer to rewardsModule.
    function harvest(address rewardsModule) external override onlyVault returns (uint256) {
        if (address(cometRewards) == address(0) || address(rewardToken) == address(0) || rewardsModule == address(0)) {
            return 0;
        }

        uint256 balBefore = rewardToken.balanceOf(address(this));
        cometRewards.claim(address(comet), address(this), true);
        uint256 claimed = rewardToken.balanceOf(address(this)) - balBefore;

        if (claimed > 0) {
            IERC20(address(rewardToken)).safeTransfer(rewardsModule, claimed);
        }
        return claimed;
    }

    /// @notice Sweep non-asset reward tokens to a recipient (e.g. RewardsModule).
    function sweepReward(address rewardToken_, address to) external override onlyVault returns (uint256 amount) {
        if (rewardToken_ == asset || rewardToken_ == address(comet)) revert CannotSweepAsset();
        amount = IERC20(rewardToken_).balanceOf(address(this));
        if (amount > 0) {
            IERC20(rewardToken_).safeTransfer(to, amount);
        }
    }
}
