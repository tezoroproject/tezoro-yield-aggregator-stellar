// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IStrategy} from "../interfaces/IStrategy.sol";
import {IAaveV3Pool} from "../interfaces/IAaveV3Pool.sol";
import {IRewardsController} from "../interfaces/IRewardsController.sol";

/// @title AaveV3Strategy
/// @notice Strategy adapter for Aave V3 lending pool.
///         Deposits and withdraws a single asset. Reports balance via aToken.
///         Harvests incentive rewards (ARB, OP, etc.) via RewardsController.
contract AaveV3Strategy is IStrategy {
    using SafeERC20 for IERC20;

    address public immutable override asset;
    IAaveV3Pool public immutable pool;
    IERC20 public immutable aToken;
    address public immutable vault;
    IRewardsController public immutable rewardsController;

    error NotVault();
    error ZeroAddress();
    error CannotSweepAsset();

    modifier onlyVault() {
        if (msg.sender != vault) revert NotVault();
        _;
    }

    /// @param rewardsController_ Aave/Spark incentives controller. address(0) = no rewards.
    constructor(address asset_, address pool_, address aToken_, address vault_, address rewardsController_) {
        if (asset_ == address(0) || pool_ == address(0) || aToken_ == address(0) || vault_ == address(0)) {
            revert ZeroAddress();
        }

        asset = asset_;
        pool = IAaveV3Pool(pool_);
        aToken = IERC20(aToken_);
        vault = vault_;
        rewardsController = IRewardsController(rewardsController_);

        // Approve pool to pull assets for supply
        IERC20(asset_).forceApprove(pool_, type(uint256).max);
    }

    function deposit(uint256 amount) external override onlyVault {
        // Pull assets from vault
        IERC20(asset).safeTransferFrom(vault, address(this), amount);
        // Supply to Aave
        pool.supply(asset, amount, address(this), 0);
    }

    function withdraw(uint256 amount) external override onlyVault returns (uint256 withdrawn) {
        uint256 available = availableLiquidity();
        uint256 toWithdraw = amount > available ? available : amount;
        if (toWithdraw == 0) return 0;

        withdrawn = pool.withdraw(asset, toWithdraw, vault);
    }

    function emergencyWithdraw() external override onlyVault returns (uint256 withdrawn) {
        uint256 balance = aToken.balanceOf(address(this));
        if (balance == 0) return 0;

        // type(uint256).max tells Aave to withdraw everything
        withdrawn = pool.withdraw(asset, type(uint256).max, vault);
    }

    function balanceOf() external view override returns (uint256) {
        return aToken.balanceOf(address(this));
    }

    function availableLiquidity() public view override returns (uint256) {
        uint256 ourBalance = aToken.balanceOf(address(this));
        uint256 poolLiquidity = IERC20(asset).balanceOf(address(aToken));
        return ourBalance > poolLiquidity ? poolLiquidity : ourBalance;
    }

    function isHealthy() external view override returns (bool) {
        // Check that the Aave pool has some liquidity for this asset
        uint256 poolLiquidity = IERC20(asset).balanceOf(address(aToken));
        return poolLiquidity > 0;
    }

    /// @notice Claim incentive rewards (ARB, OP, etc.) and send to rewardsModule.
    ///         Aave's claimAllRewards sends directly to the target -- no intermediate holding.
    function harvest(address rewardsModule) external override onlyVault returns (uint256) {
        if (address(rewardsController) == address(0) || rewardsModule == address(0)) return 0;

        address[] memory assets = new address[](1);
        assets[0] = address(aToken);

        (, uint256[] memory amounts) = rewardsController.claimAllRewards(assets, rewardsModule);

        uint256 total = 0;
        for (uint256 i = 0; i < amounts.length; i++) {
            total += amounts[i];
        }
        return total;
    }

    /// @notice Sweep non-asset reward tokens to a recipient (e.g. RewardsModule).
    function sweepReward(address rewardToken, address to) external override onlyVault returns (uint256 amount) {
        if (rewardToken == asset || rewardToken == address(aToken)) revert CannotSweepAsset();
        amount = IERC20(rewardToken).balanceOf(address(this));
        if (amount > 0) {
            IERC20(rewardToken).safeTransfer(to, amount);
        }
    }
}
