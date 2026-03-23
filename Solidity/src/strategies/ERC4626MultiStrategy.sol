// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC4626} from "@openzeppelin/contracts/interfaces/IERC4626.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IStrategy} from "../interfaces/IStrategy.sol";

/// @title ERC4626MultiStrategy
/// @notice Strategy adapter that wraps multiple whitelisted ERC-4626 vaults.
///         The vault calls standard IStrategy methods (deposit/withdraw).
///         A keeper allocates/deallocates idle funds across sub-vaults.
contract ERC4626MultiStrategy is IStrategy, ReentrancyGuard {
    using SafeERC20 for IERC20;

    uint256 public immutable MAX_SUB_VAULTS;

    address public immutable override asset;
    address public immutable vault;
    string public name;

    address public admin;
    address public pendingAdmin;
    address public keeper;

    address[] internal _subVaults;
    mapping(address => bool) public isApproved;
    mapping(address => bool) public depositFrozenSubVaults;

    error NotVault();
    error NotAdmin();
    error NotKeeper();
    error ZeroAddress();
    error CannotSweepAsset();
    error SubVaultAlreadyApproved();
    error SubVaultNotApproved();
    error AssetMismatch();
    error InsufficientIdle();
    error SubVaultAlreadyDepositFrozen();
    error SubVaultNotDepositFrozen();
    error DepositsFrozen();
    error SubVaultHasActivePosition();
    error NotPendingAdmin();
    error TooManySubVaults();

    event SubVaultAdded(address indexed subVault);
    event SubVaultRemoved(address indexed subVault);
    event SubVaultDepositFrozen(address indexed subVault);
    event SubVaultDepositUnfrozen(address indexed subVault);
    event SubVaultRecalled(address indexed subVault, uint256 assets);
    event KeeperSet(address indexed keeper);
    event AdminTransferProposed(address indexed currentAdmin, address indexed proposedAdmin);
    event AdminTransferred(address indexed previousAdmin, address indexed newAdmin);
    event Allocated(address indexed subVault, uint256 assets, uint256 shares);
    event Deallocated(address indexed subVault, uint256 assets);
    event WithdrawFailed(address indexed subVault);

    modifier onlyVault() {
        if (msg.sender != vault) revert NotVault();
        _;
    }

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    modifier onlyKeeper() {
        if (msg.sender != keeper) revert NotKeeper();
        _;
    }

    error MaxSubVaultsTooLow();

    constructor(
        address asset_,
        address vault_,
        address admin_,
        string memory name_,
        uint256 maxSubVaults_,
        address[] memory initialSubVaults_
    ) {
        if (asset_ == address(0) || vault_ == address(0) || admin_ == address(0)) {
            revert ZeroAddress();
        }
        if (maxSubVaults_ == 0) revert MaxSubVaultsTooLow();
        asset = asset_;
        vault = vault_;
        admin = admin_;
        name = name_;
        MAX_SUB_VAULTS = maxSubVaults_;

        // Seed initial whitelist (same validation as addSubVault, but no onlyAdmin check)
        if (initialSubVaults_.length > maxSubVaults_) revert TooManySubVaults();
        for (uint256 i = 0; i < initialSubVaults_.length; i++) {
            address sv = initialSubVaults_[i];
            if (sv == address(0)) revert ZeroAddress();
            if (isApproved[sv]) revert SubVaultAlreadyApproved();
            if (IERC4626(sv).asset() != asset_) revert AssetMismatch();

            isApproved[sv] = true;
            _subVaults.push(sv);
            IERC20(asset_).forceApprove(sv, type(uint256).max);

            emit SubVaultAdded(sv);
        }
    }

    // ========== IStrategy (vault calls) ==========

    function deposit(uint256 amount) external override onlyVault nonReentrant {
        IERC20(asset).safeTransferFrom(vault, address(this), amount);
        // Funds stay idle until keeper allocates
    }

    function withdraw(uint256 amount) external override onlyVault nonReentrant returns (uint256 withdrawn) {
        uint256 idle = IERC20(asset).balanceOf(address(this));

        // Use idle first
        if (idle >= amount) {
            IERC20(asset).safeTransfer(vault, amount);
            return amount;
        }

        // Send all idle
        if (idle > 0) {
            IERC20(asset).safeTransfer(vault, idle);
            withdrawn = idle;
        }

        uint256 remaining = amount - withdrawn;

        // Waterfall through sub-vaults (try-catch so one broken sub-vault cannot block all withdrawals)
        uint256 len = _subVaults.length;
        for (uint256 i = 0; i < len; i++) {
            if (remaining == 0) break;

            IERC4626 sv = IERC4626(_subVaults[i]);
            try sv.maxWithdraw(address(this)) returns (uint256 maxW) {
                if (maxW == 0) continue;

                uint256 toWithdraw = remaining > maxW ? maxW : remaining;
                // Measure actual delivery — sub-vaults with exit fees may send less than requested
                uint256 vaultBalBefore = IERC20(asset).balanceOf(vault);
                try sv.withdraw(toWithdraw, vault, address(this)) {
                    uint256 got = IERC20(asset).balanceOf(vault) - vaultBalBefore;
                    withdrawn += got;
                    remaining = remaining > got ? remaining - got : 0;
                } catch {
                    // Fallback: some vaults (e.g., YO.xyz) only support redeem, not withdraw.
                    // Convert requested assets to shares and redeem instead.
                    uint256 shares = sv.convertToShares(toWithdraw);
                    if (shares == 0) {
                        emit WithdrawFailed(address(sv));
                        continue;
                    }
                    try sv.redeem(shares, vault, address(this)) {
                        uint256 got = IERC20(asset).balanceOf(vault) - vaultBalBefore;
                        withdrawn += got;
                        remaining = remaining > got ? remaining - got : 0;
                    } catch {
                        emit WithdrawFailed(address(sv));
                        continue;
                    }
                }
            } catch {
                emit WithdrawFailed(address(sv));
                continue;
            }
        }
    }

    function emergencyWithdraw() external override onlyVault nonReentrant returns (uint256 withdrawn) {
        // Redeem all shares from all sub-vaults (try-catch: recover as much as possible)
        uint256 len = _subVaults.length;
        for (uint256 i = 0; i < len; i++) {
            IERC4626 sv = IERC4626(_subVaults[i]);
            try sv.balanceOf(address(this)) returns (uint256 shares) {
                if (shares == 0) continue;
                try sv.redeem(shares, address(this), address(this)) {} catch {
                    emit WithdrawFailed(address(sv));
                }
            } catch {
                emit WithdrawFailed(address(sv));
            }
        }

        // Send everything (redeemed + idle) to vault
        withdrawn = IERC20(asset).balanceOf(address(this));
        if (withdrawn > 0) {
            IERC20(asset).safeTransfer(vault, withdrawn);
        }
    }

    function balanceOf() external view override returns (uint256 total) {
        total = IERC20(asset).balanceOf(address(this));

        uint256 len = _subVaults.length;
        for (uint256 i = 0; i < len; i++) {
            IERC4626 sv = IERC4626(_subVaults[i]);
            uint256 shares = sv.balanceOf(address(this));
            if (shares > 0) {
                total += sv.convertToAssets(shares);
            }
        }
    }

    function availableLiquidity() public view override returns (uint256 total) {
        total = IERC20(asset).balanceOf(address(this));

        uint256 len = _subVaults.length;
        for (uint256 i = 0; i < len; i++) {
            total += IERC4626(_subVaults[i]).maxWithdraw(address(this));
        }
    }

    function isHealthy() external view override returns (bool) {
        uint256 len = _subVaults.length;
        for (uint256 i = 0; i < len; i++) {
            if (IERC4626(_subVaults[i]).totalAssets() > 0) return true;
        }
        return false;
    }

    function harvest(address) external pure override returns (uint256) {
        // Rewards are handled externally (Merkl, URD, etc.)
        return 0;
    }

    function sweepReward(address rewardToken, address to) external override onlyVault returns (uint256 amount) {
        if (rewardToken == asset) revert CannotSweepAsset();
        if (isApproved[rewardToken]) revert CannotSweepAsset();
        amount = IERC20(rewardToken).balanceOf(address(this));
        if (amount > 0) {
            IERC20(rewardToken).safeTransfer(to, amount);
        }
    }

    // ========== Keeper operations ==========

    function allocate(address subVault, uint256 amount) external onlyKeeper nonReentrant {
        if (!isApproved[subVault]) revert SubVaultNotApproved();
        if (depositFrozenSubVaults[subVault]) revert DepositsFrozen();
        uint256 idle = IERC20(asset).balanceOf(address(this));
        if (amount > idle) revert InsufficientIdle();

        uint256 shares = IERC4626(subVault).deposit(amount, address(this));
        emit Allocated(subVault, amount, shares);
    }

    function deallocate(address subVault, uint256 amount) external onlyKeeper nonReentrant {
        if (!isApproved[subVault]) revert SubVaultNotApproved();

        // Try withdraw first; fall back to redeem for vaults that only support redeem (e.g., YO.xyz)
        try IERC4626(subVault).withdraw(amount, address(this), address(this)) {}
        catch {
            uint256 shares = IERC4626(subVault).convertToShares(amount);
            IERC4626(subVault).redeem(shares, address(this), address(this));
        }
        emit Deallocated(subVault, amount);
    }

    // ========== Admin operations ==========

    function addSubVault(address subVault) external onlyAdmin {
        if (subVault == address(0)) revert ZeroAddress();
        if (_subVaults.length >= MAX_SUB_VAULTS) revert TooManySubVaults();
        if (isApproved[subVault]) revert SubVaultAlreadyApproved();
        if (IERC4626(subVault).asset() != asset) revert AssetMismatch();

        isApproved[subVault] = true;
        _subVaults.push(subVault);
        IERC20(asset).forceApprove(subVault, type(uint256).max);

        emit SubVaultAdded(subVault);
    }

    function removeSubVault(address subVault) external onlyAdmin {
        if (!isApproved[subVault]) revert SubVaultNotApproved();
        if (IERC4626(subVault).balanceOf(address(this)) > 0) revert SubVaultHasActivePosition();

        isApproved[subVault] = false;
        depositFrozenSubVaults[subVault] = false;
        IERC20(asset).forceApprove(subVault, 0);

        // Swap-and-pop removal
        uint256 len = _subVaults.length;
        for (uint256 i = 0; i < len; i++) {
            if (_subVaults[i] == subVault) {
                _subVaults[i] = _subVaults[len - 1];
                _subVaults.pop();
                break;
            }
        }

        emit SubVaultRemoved(subVault);
    }

    function freezeSubVaultDeposits(address subVault) external onlyAdmin {
        if (!isApproved[subVault]) revert SubVaultNotApproved();
        if (depositFrozenSubVaults[subVault]) revert SubVaultAlreadyDepositFrozen();
        depositFrozenSubVaults[subVault] = true;
        emit SubVaultDepositFrozen(subVault);
    }

    function unfreezeSubVaultDeposits(address subVault) external onlyAdmin {
        if (!isApproved[subVault]) revert SubVaultNotApproved();
        if (!depositFrozenSubVaults[subVault]) revert SubVaultNotDepositFrozen();
        depositFrozenSubVaults[subVault] = false;
        emit SubVaultDepositUnfrozen(subVault);
    }

    /// @notice Redeem all shares from a sub-vault back to idle and freeze deposits.
    function recallSubVault(address subVault) external onlyAdmin nonReentrant {
        if (!isApproved[subVault]) revert SubVaultNotApproved();

        uint256 recalled = 0;
        try IERC4626(subVault).balanceOf(address(this)) returns (uint256 shares) {
            if (shares > 0) {
                try IERC4626(subVault).redeem(shares, address(this), address(this)) returns (uint256 got) {
                    recalled = got;
                } catch {
                    emit WithdrawFailed(subVault);
                }
            }
        } catch {
            emit WithdrawFailed(subVault);
        }

        if (!depositFrozenSubVaults[subVault]) {
            depositFrozenSubVaults[subVault] = true;
            emit SubVaultDepositFrozen(subVault);
        }

        emit SubVaultRecalled(subVault, recalled);
    }

    function setKeeper(address keeper_) external onlyAdmin {
        if (keeper_ == address(0)) revert ZeroAddress();
        keeper = keeper_;
        emit KeeperSet(keeper_);
    }

    function transferAdmin(address newAdmin) external onlyAdmin {
        if (newAdmin == address(0)) revert ZeroAddress();
        pendingAdmin = newAdmin;
        emit AdminTransferProposed(admin, newAdmin);
    }

    function acceptAdmin() external {
        if (msg.sender != pendingAdmin) revert NotPendingAdmin();
        emit AdminTransferred(admin, pendingAdmin);
        admin = pendingAdmin;
        pendingAdmin = address(0);
    }

    // ========== View helpers ==========

    function getSubVaults() external view returns (address[] memory) {
        return _subVaults;
    }

    function subVaultBalance(address subVault) external view returns (uint256) {
        uint256 shares = IERC4626(subVault).balanceOf(address(this));
        if (shares == 0) return 0;
        return IERC4626(subVault).convertToAssets(shares);
    }

    function subVaultCount() external view returns (uint256) {
        return _subVaults.length;
    }
}
