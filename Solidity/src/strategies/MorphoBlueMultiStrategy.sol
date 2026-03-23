// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IStrategy} from "../interfaces/IStrategy.sol";
import {IMorpho, MarketParams, MorphoMarket, Position, Id} from "../interfaces/IMorpho.sol";

/// @title MorphoBlueMultiStrategy
/// @notice Strategy adapter that wraps multiple whitelisted Morpho Blue lending markets.
///         The vault calls standard IStrategy methods (deposit/withdraw).
///         A keeper allocates/deallocates idle funds across markets.
contract MorphoBlueMultiStrategy is IStrategy, ReentrancyGuard {
    using SafeERC20 for IERC20;
    using Math for uint256;

    uint256 public immutable MAX_MARKETS;

    address public immutable override asset;
    address public immutable vault;
    IMorpho public immutable morpho;
    string public name;

    address public admin;
    address public pendingAdmin;
    address public keeper;

    Id[] internal _marketIds;
    mapping(Id => bool) public isApproved;
    mapping(Id => bool) public depositFrozenMarkets;
    mapping(Id => MarketParams) internal _marketParams;

    error NotVault();
    error NotAdmin();
    error NotKeeper();
    error ZeroAddress();
    error CannotSweepAsset();
    error MarketAlreadyApproved();
    error MarketNotApproved();
    error LoanTokenMismatch();
    error InsufficientIdle();
    error MarketAlreadyDepositFrozen();
    error MarketNotDepositFrozen();
    error DepositsFrozen();
    error MarketHasActivePosition();
    error NotPendingAdmin();
    error TooManyMarkets();

    event MarketAdded(Id indexed marketId);
    event MarketRemoved(Id indexed marketId);
    event MarketDepositFrozen(Id indexed marketId);
    event MarketDepositUnfrozen(Id indexed marketId);
    event MarketRecalled(Id indexed marketId, uint256 assets);
    event KeeperSet(address indexed keeper);
    event AdminTransferProposed(address indexed currentAdmin, address indexed proposedAdmin);
    event AdminTransferred(address indexed previousAdmin, address indexed newAdmin);
    event Allocated(Id indexed marketId, uint256 assets, uint256 shares);
    event Deallocated(Id indexed marketId, uint256 assets);
    event WithdrawFailed(Id indexed marketId);

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

    error MaxMarketsTooLow();

    constructor(
        address asset_,
        address morpho_,
        address vault_,
        address admin_,
        string memory name_,
        uint256 maxMarkets_,
        MarketParams[] memory initialMarkets_
    ) {
        if (asset_ == address(0) || morpho_ == address(0) || vault_ == address(0) || admin_ == address(0)) {
            revert ZeroAddress();
        }
        if (maxMarkets_ == 0) revert MaxMarketsTooLow();
        asset = asset_;
        morpho = IMorpho(morpho_);
        vault = vault_;
        admin = admin_;
        name = name_;
        MAX_MARKETS = maxMarkets_;

        // Approve Morpho to pull assets for supply
        IERC20(asset_).forceApprove(morpho_, type(uint256).max);

        // Seed initial whitelist (same validation as addMarket, but no onlyAdmin check)
        if (initialMarkets_.length > maxMarkets_) revert TooManyMarkets();
        for (uint256 i = 0; i < initialMarkets_.length; i++) {
            MarketParams memory mp = initialMarkets_[i];
            if (mp.loanToken != asset_) revert LoanTokenMismatch();

            Id mid = Id.wrap(keccak256(abi.encode(mp)));
            if (isApproved[mid]) revert MarketAlreadyApproved();

            isApproved[mid] = true;
            _marketIds.push(mid);
            _marketParams[mid] = mp;

            emit MarketAdded(mid);
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

        if (idle > 0) {
            IERC20(asset).safeTransfer(vault, idle);
            withdrawn = idle;
        }

        uint256 remaining = amount - withdrawn;

        // Waterfall through markets (try-catch so one broken market cannot block all withdrawals)
        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            if (remaining == 0) break;

            Id mid = _marketIds[i];
            Position memory pos = morpho.position(mid, address(this));
            if (pos.supplyShares == 0) continue;

            uint256 ourBalance = _supplyAssets(mid);
            uint256 poolLiquidity = _poolLiquidity(mid);
            uint256 available = ourBalance > poolLiquidity ? poolLiquidity : ourBalance;
            uint256 toWithdraw = remaining > available ? available : remaining;
            if (toWithdraw == 0) continue;

            try morpho.withdraw(
                _marketParams[mid],
                toWithdraw >= ourBalance ? 0 : toWithdraw,
                toWithdraw >= ourBalance ? pos.supplyShares : 0,
                address(this),
                vault
            ) returns (uint256 got, uint256) {
                withdrawn += got;
                remaining = remaining > got ? remaining - got : 0;
            } catch {
                emit WithdrawFailed(mid);
                continue;
            }
        }
    }

    function emergencyWithdraw() external override onlyVault nonReentrant returns (uint256 withdrawn) {
        // Withdraw all shares from all markets (try-catch: recover as much as possible)
        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            Id mid = _marketIds[i];
            Position memory pos = morpho.position(mid, address(this));
            if (pos.supplyShares == 0) continue;

            try morpho.withdraw(_marketParams[mid], 0, pos.supplyShares, address(this), address(this)) {} catch {
                emit WithdrawFailed(mid);
            }
        }

        // Send everything (recovered + idle) to vault
        withdrawn = IERC20(asset).balanceOf(address(this));
        if (withdrawn > 0) {
            IERC20(asset).safeTransfer(vault, withdrawn);
        }
    }

    function balanceOf() external view override returns (uint256 total) {
        total = IERC20(asset).balanceOf(address(this));

        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            Id mid = _marketIds[i];
            Position memory pos = morpho.position(mid, address(this));
            if (pos.supplyShares == 0) continue;

            total += _supplyAssets(mid);
        }
    }

    function availableLiquidity() public view override returns (uint256 total) {
        total = IERC20(asset).balanceOf(address(this));

        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            Id mid = _marketIds[i];
            Position memory pos = morpho.position(mid, address(this));
            if (pos.supplyShares == 0) continue;

            uint256 ourBalance = _supplyAssets(mid);
            uint256 poolLiquidity = _poolLiquidity(mid);
            total += ourBalance > poolLiquidity ? poolLiquidity : ourBalance;
        }
    }

    function isHealthy() external view override returns (bool) {
        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            MorphoMarket memory m = morpho.market(_marketIds[i]);
            if (m.totalSupplyShares > 0) return true;
        }
        return false;
    }

    function harvest(address) external pure override returns (uint256) {
        // Morpho rewards via URD — claims done externally, then swept
        return 0;
    }

    function sweepReward(address rewardToken, address to) external override onlyVault returns (uint256 amount) {
        if (rewardToken == asset) revert CannotSweepAsset();
        amount = IERC20(rewardToken).balanceOf(address(this));
        if (amount > 0) {
            IERC20(rewardToken).safeTransfer(to, amount);
        }
    }

    // ========== Keeper operations ==========

    /// @notice Accrue interest on all markets so that balanceOf() returns up-to-date values.
    ///         Call before vault.reconcile() to ensure tracked balances reflect accrued yield.
    function accrueAllInterest() external {
        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            morpho.accrueInterest(_marketParams[_marketIds[i]]);
        }
    }

    function allocate(Id marketId, uint256 amount) external onlyKeeper nonReentrant {
        if (!isApproved[marketId]) revert MarketNotApproved();
        if (depositFrozenMarkets[marketId]) revert DepositsFrozen();
        uint256 idle = IERC20(asset).balanceOf(address(this));
        if (amount > idle) revert InsufficientIdle();

        (, uint256 shares) = morpho.supply(_marketParams[marketId], amount, 0, address(this), "");
        emit Allocated(marketId, amount, shares);
    }

    function deallocate(Id marketId, uint256 amount) external onlyKeeper nonReentrant {
        if (!isApproved[marketId]) revert MarketNotApproved();

        (uint256 withdrawn,) = morpho.withdraw(_marketParams[marketId], amount, 0, address(this), address(this));
        emit Deallocated(marketId, withdrawn);
    }

    // ========== Admin operations ==========

    function addMarket(MarketParams memory mp) external onlyAdmin {
        if (mp.loanToken != asset) revert LoanTokenMismatch();
        if (_marketIds.length >= MAX_MARKETS) revert TooManyMarkets();

        Id mid = Id.wrap(keccak256(abi.encode(mp)));
        if (isApproved[mid]) revert MarketAlreadyApproved();

        isApproved[mid] = true;
        _marketIds.push(mid);
        _marketParams[mid] = mp;

        emit MarketAdded(mid);
    }

    function removeMarket(Id marketId) external onlyAdmin {
        if (!isApproved[marketId]) revert MarketNotApproved();
        Position memory pos = morpho.position(marketId, address(this));
        if (pos.supplyShares > 0) revert MarketHasActivePosition();

        isApproved[marketId] = false;
        depositFrozenMarkets[marketId] = false;
        delete _marketParams[marketId];

        // Swap-and-pop removal
        uint256 len = _marketIds.length;
        for (uint256 i = 0; i < len; i++) {
            if (Id.unwrap(_marketIds[i]) == Id.unwrap(marketId)) {
                _marketIds[i] = _marketIds[len - 1];
                _marketIds.pop();
                break;
            }
        }

        emit MarketRemoved(marketId);
    }

    function freezeMarketDeposits(Id marketId) external onlyAdmin {
        if (!isApproved[marketId]) revert MarketNotApproved();
        if (depositFrozenMarkets[marketId]) revert MarketAlreadyDepositFrozen();
        depositFrozenMarkets[marketId] = true;
        emit MarketDepositFrozen(marketId);
    }

    function unfreezeMarketDeposits(Id marketId) external onlyAdmin {
        if (!isApproved[marketId]) revert MarketNotApproved();
        if (!depositFrozenMarkets[marketId]) revert MarketNotDepositFrozen();
        depositFrozenMarkets[marketId] = false;
        emit MarketDepositUnfrozen(marketId);
    }

    /// @notice Withdraw all funds from a market back to idle and freeze deposits.
    function recallMarket(Id marketId) external onlyAdmin nonReentrant {
        if (!isApproved[marketId]) revert MarketNotApproved();

        Position memory pos = morpho.position(marketId, address(this));
        uint256 recalled = 0;
        if (pos.supplyShares > 0) {
            try morpho.withdraw(_marketParams[marketId], 0, pos.supplyShares, address(this), address(this))
                returns (uint256 got, uint256)
            {
                recalled = got;
            } catch {
                emit WithdrawFailed(marketId);
            }
        }

        if (!depositFrozenMarkets[marketId]) {
            depositFrozenMarkets[marketId] = true;
            emit MarketDepositFrozen(marketId);
        }

        emit MarketRecalled(marketId, recalled);
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

    // ========== Internal helpers ==========

    /// @dev Compute supply assets from shares using on-chain market state.
    ///      Uses last-accrued totals (no interest projection) — accurate to within
    ///      seconds of stale interest, which is acceptable for accounting.
    function _supplyAssets(Id mid) internal view returns (uint256) {
        Position memory pos = morpho.position(mid, address(this));
        if (pos.supplyShares == 0) return 0;
        MorphoMarket memory m = morpho.market(mid);
        if (m.totalSupplyShares == 0) return 0;
        return uint256(pos.supplyShares).mulDiv(uint256(m.totalSupplyAssets), uint256(m.totalSupplyShares));
    }

    /// @dev Pool liquidity = totalSupplyAssets - totalBorrowAssets (last-accrued).
    function _poolLiquidity(Id mid) internal view returns (uint256) {
        MorphoMarket memory m = morpho.market(mid);
        if (m.totalBorrowAssets >= m.totalSupplyAssets) return 0;
        return uint256(m.totalSupplyAssets) - uint256(m.totalBorrowAssets);
    }

    // ========== View helpers ==========

    function getMarketIds() external view returns (Id[] memory) {
        return _marketIds;
    }

    function getMarketParams(Id marketId) external view returns (MarketParams memory) {
        return _marketParams[marketId];
    }

    function marketBalance(Id marketId) external view returns (uint256) {
        Position memory pos = morpho.position(marketId, address(this));
        if (pos.supplyShares == 0) return 0;

        return _supplyAssets(marketId);
    }

    function marketCount() external view returns (uint256) {
        return _marketIds.length;
    }
}
