// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import {ERC4626} from "@openzeppelin/contracts/token/ERC20/extensions/ERC4626.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IStrategy} from "./interfaces/IStrategy.sol";

/// @title TezoroV1
/// @notice ERC-4626 vault that deploys assets across multiple lending strategies.
///         Supports idle buffer, withdrawal waterfall, HWM-based performance fee,
///         keeper role, rewards auto-compounding, and admin-managed strategy allocation.
/// @dev Internal accounting prevents donation attacks. Virtual shares offset (via _decimalsOffset)
///      prevents first-depositor inflation attack.
contract TezoroV1_1 is ERC4626, ReentrancyGuard {
    using SafeERC20 for IERC20;
    using Math for uint256;

    // --- Constants ---

    uint256 public constant MAX_STRATEGIES = 20;
    uint256 public constant BPS = 10_000;
    uint256 public constant MAX_PERFORMANCE_FEE_BPS = 3_000; // 30% cap
    uint256 public constant MAX_IDLE_BUFFER_BPS = 2_000; // 20% cap
    uint256 public constant MAX_FORCE_REDEEM_BATCH = 50;

    // --- Immutables ---

    uint8 private immutable _assetDecimals;

    // --- State: Strategies ---

    IStrategy[] public strategies;
    mapping(IStrategy => bool) public isActiveStrategy;
    mapping(IStrategy => uint256) public targetAllocationBps; // target % of total assets per strategy
    mapping(IStrategy => uint256) public trackedBalance; // internal accounting per strategy
    mapping(IStrategy => bool) public pausedStrategies;
    mapping(IStrategy => bool) public depositFrozenStrategies; // blocks deposits only, withdrawals still work
    mapping(IStrategy => uint256) public maxAllocationBps; // per-strategy cap (0 = no cap)

    // --- State: Fees ---

    uint256 public performanceFeeBps;
    uint256 public highWaterMark; // share price HWM for performance fee
    address public feeRecipient;

    // --- State: Config ---

    uint256 public idleBufferBps; // target idle buffer as % of total assets
    uint256 public maxDeviationBps; // rebalance trigger threshold (0 = no threshold check)
    uint256 public depositCap; // max total assets allowed (0 = no cap)
    address public admin;
    address public pendingAdmin;
    address public keeper; // separate role for automated operations
    address public guardian; // can pause, cannot unpause
    bool public paused;
    address public rewardsModule; // address(0) = disabled

    // --- State: Timelock stub ---

    struct TimelockProposal {
        bytes32 operationHash;
        uint256 readyTimestamp;
        bool executed;
    }

    uint256 public timelockDelay; // 0 = timelock disabled (MVP default)
    mapping(bytes32 => TimelockProposal) public timelockProposals;

    // --- Events ---

    event StrategyAdded(address indexed strategy);
    event StrategyRemoved(address indexed strategy);
    event StrategyPaused(address indexed strategy);
    event StrategyUnpaused(address indexed strategy);
    event AllocationSet(address indexed strategy, uint256 bps);
    event Rebalanced();
    event Reconciled();
    event PerformanceFeeCollected(uint256 fee, address recipient);
    event AdminTransferProposed(address indexed currentAdmin, address indexed proposedAdmin);
    event AdminTransferred(address indexed previousAdmin, address indexed newAdmin);
    event KeeperUpdated(address indexed previousKeeper, address indexed newKeeper);
    event GuardianUpdated(address indexed previousGuardian, address indexed newGuardian);
    event MaxAllocationSet(address indexed strategy, uint256 maxBps);
    event IdleBufferUpdated(uint256 oldBps, uint256 newBps);
    event MaxDeviationUpdated(uint256 oldBps, uint256 newBps);
    event VaultPaused();
    event VaultUnpaused();
    event RewardsModuleUpdated(address indexed oldModule, address indexed newModule);
    event RewardsDeposited(address indexed from, uint256 amount);
    event HarvestCompleted(uint256 totalHarvested);
    event PerformanceFeeUpdated(uint256 oldBps, uint256 newBps);
    event FeeRecipientUpdated(address indexed oldRecipient, address indexed newRecipient);
    event TimelockDelayUpdated(uint256 oldDelay, uint256 newDelay);
    event TimelockProposed(bytes32 indexed operationHash, uint256 readyTimestamp);
    event TimelockCancelled(bytes32 indexed operationHash);
    event DepositCapUpdated(uint256 oldCap, uint256 newCap);
    event StrategyRewardSwept(address indexed strategy, address indexed rewardToken, address indexed to, uint256 amount);
    event StrategyDepositFrozen(address indexed strategy);
    event StrategyDepositUnfrozen(address indexed strategy);
    event RecalledToIdle(address indexed strategy, uint256 amount);
    event RecallFailed(address indexed strategy, uint256 trackedAmount);
    event StrategyRemovalFundsLost(address indexed strategy, uint256 lostAmount);
    event ForceRedeemed(address indexed user, uint256 shares, uint256 assets);

    // --- Errors ---

    error NotAdmin();
    error NotPendingAdmin();
    error NotAdminOrKeeper();
    error NotGuardianOrAdmin();
    error NotRewardsModule();
    error VaultIsPaused();
    error StrategyAlreadyActive();
    error StrategyNotActive();
    error StrategyNotHealthy();
    error TooManyStrategies();
    error InvalidAllocation();
    error AllocationExceedsCap();
    error InvalidFee();
    error InvalidBuffer();
    error ZeroAddress();
    error StrategyAssetMismatch();
    error WithdrawalFailed();
    error TimelockNotFound();
    error StrategyAlreadyDepositFrozen();
    error StrategyNotDepositFrozen();
    error NoSharesToRedeem();
    error EmptyUserList();
    error BatchTooLarge();
    error TimelockNotReady();
    error TimelockAlreadyExecuted();
    error TimelockAlreadyPending();

    // --- Modifiers ---

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    modifier onlyAdminOrKeeper() {
        if (msg.sender != admin && msg.sender != keeper) revert NotAdminOrKeeper();
        _;
    }

    modifier onlyGuardianOrAdmin() {
        if (msg.sender != guardian && msg.sender != admin) revert NotGuardianOrAdmin();
        _;
    }

    modifier whenNotPaused() {
        if (paused) revert VaultIsPaused();
        _;
    }

    // --- Constructor ---

    constructor(
        IERC20 asset_,
        string memory name_,
        string memory symbol_,
        address admin_,
        address feeRecipient_,
        uint256 performanceFeeBps_,
        uint256 idleBufferBps_
    ) ERC4626(asset_) ERC20(name_, symbol_) {
        if (admin_ == address(0)) revert ZeroAddress();
        if (feeRecipient_ == address(0)) revert ZeroAddress();
        if (performanceFeeBps_ > MAX_PERFORMANCE_FEE_BPS) revert InvalidFee();
        if (idleBufferBps_ > MAX_IDLE_BUFFER_BPS) revert InvalidBuffer();

        _assetDecimals = ERC20(asset()).decimals();
        admin = admin_;
        feeRecipient = feeRecipient_;
        performanceFeeBps = performanceFeeBps_;
        idleBufferBps = idleBufferBps_;
        // HWM = initial share price so first collectFees() doesn't charge phantom perf fee
        highWaterMark = convertToAssets(10 ** decimals());
    }

    // =========================================================================
    // ERC-4626 Overrides
    // =========================================================================

    /// @notice Total assets = idle balance + sum of tracked strategy balances.
    ///         Strategy balances use internal accounting (not live queries) to prevent
    ///         donation-based manipulation. Idle balance uses live balanceOf; the virtual
    ///         shares offset (_decimalsOffset) makes idle-donation attacks economically infeasible.
    function totalAssets() public view override returns (uint256) {
        uint256 total = IERC20(asset()).balanceOf(address(this));
        for (uint256 i = 0; i < strategies.length; i++) {
            if (!pausedStrategies[strategies[i]]) {
                total += trackedBalance[strategies[i]];
            }
        }
        return total;
    }

    /// @notice Virtual shares offset to prevent first-depositor inflation attack.
    ///         Uses asset decimals so the offset scales with the token's precision.
    function _decimalsOffset() internal view override returns (uint8) {
        return _assetDecimals;
    }

    function maxDeposit(address) public view override returns (uint256) {
        if (paused) return 0;
        if (depositCap == 0) return type(uint256).max;
        uint256 total = totalAssets();
        if (total >= depositCap) return 0;
        return depositCap - total;
    }

    function maxMint(address) public view override returns (uint256) {
        if (paused) return 0;
        if (depositCap == 0) return type(uint256).max;
        uint256 total = totalAssets();
        if (total >= depositCap) return 0;
        return previewDeposit(depositCap - total);
    }

    /// @dev maxWithdraw/maxRedeem account for pending performance fee dilution so that
    ///      withdraw(maxWithdraw(x)) and redeem(maxRedeem(x)) never revert (ERC-4626 compliance).
    ///      Without this, _accruePerformanceFee() inside withdraw/redeem mints fee shares that
    ///      dilute the caller, making the pre-accrual maxWithdraw/maxRedeem values too optimistic.
    function maxWithdraw(address owner) public view override returns (uint256) {
        uint256 ownerShares = balanceOf(owner);
        uint256 pendingShares = _pendingFeeShares();
        uint256 offset = 10 ** _decimalsOffset();
        uint256 ownerAssets = ownerShares.mulDiv(
            totalAssets() + 1, totalSupply() + pendingShares + offset, Math.Rounding.Floor
        );
        uint256 available = _availableLiquidity();
        return ownerAssets > available ? available : ownerAssets;
    }

    function maxRedeem(address owner) public view override returns (uint256) {
        uint256 ownerShares = balanceOf(owner);
        uint256 pendingShares = _pendingFeeShares();
        uint256 offset = 10 ** _decimalsOffset();
        uint256 adjustedSupply = totalSupply() + pendingShares + offset;
        uint256 total = totalAssets() + 1;
        uint256 ownerAssets = ownerShares.mulDiv(total, adjustedSupply, Math.Rounding.Floor);
        uint256 available = _availableLiquidity();
        if (ownerAssets <= available) return ownerShares;
        return available.mulDiv(adjustedSupply, total, Math.Rounding.Floor);
    }

    /// @dev previewDeposit/previewMint account for pending performance fee shares so that
    ///      previewDeposit(x) == deposit(x) after deposit() accrues fees first (ERC-4626 compliance).
    ///      Without this, previewDeposit would not include the pending fee shares that are minted
    ///      at the start of deposit(), causing preview to be optimistic (returns too many shares).
    ///      Mirrors the _pendingFeeShares() pattern used by maxWithdraw/maxRedeem above.
    function previewDeposit(uint256 assets) public view override returns (uint256) {
        uint256 pendingShares = _pendingFeeShares();
        uint256 offset = 10 ** _decimalsOffset();
        return assets.mulDiv(totalSupply() + pendingShares + offset, totalAssets() + 1, Math.Rounding.Floor);
    }

    function previewMint(uint256 shares) public view override returns (uint256) {
        uint256 pendingShares = _pendingFeeShares();
        uint256 offset = 10 ** _decimalsOffset();
        return shares.mulDiv(totalAssets() + 1, totalSupply() + pendingShares + offset, Math.Rounding.Ceil);
    }

    /// @dev ERC-4626: previewWithdraw MUST return >= actual shares burned (spec: "no fewer than exact").
    ///      Without this override, pending fee dilution causes the view to return fewer shares than
    ///      will actually be burned — violating the spec and breaking delegated withdrawal allowances.
    function previewWithdraw(uint256 assets) public view override returns (uint256) {
        uint256 pendingShares = _pendingFeeShares();
        uint256 offset = 10 ** _decimalsOffset();
        return assets.mulDiv(totalSupply() + pendingShares + offset, totalAssets() + 1, Math.Rounding.Ceil);
    }

    /// @dev ERC-4626: previewRedeem MUST return <= actual assets received (spec: "no more than exact").
    ///      Without this override, pending fee dilution causes the view to return more assets than
    ///      will actually be received — violating the spec and misleading redeemers.
    function previewRedeem(uint256 shares) public view override returns (uint256) {
        uint256 pendingShares = _pendingFeeShares();
        uint256 offset = 10 ** _decimalsOffset();
        return shares.mulDiv(totalAssets() + 1, totalSupply() + pendingShares + offset, Math.Rounding.Floor);
    }

    function deposit(
        uint256 assets,
        address receiver
    ) public override nonReentrant whenNotPaused returns (uint256) {
        _accruePerformanceFee();
        return super.deposit(assets, receiver);
    }

    function mint(
        uint256 shares,
        address receiver
    ) public override nonReentrant whenNotPaused returns (uint256) {
        _accruePerformanceFee();
        return super.mint(shares, receiver);
    }

    function withdraw(
        uint256 assets,
        address receiver,
        address owner
    ) public override nonReentrant returns (uint256) {
        _accruePerformanceFee();
        return super.withdraw(assets, receiver, owner);
    }

    function redeem(
        uint256 shares,
        address receiver,
        address owner
    ) public override nonReentrant returns (uint256) {
        _accruePerformanceFee();
        return super.redeem(shares, receiver, owner);
    }

    /// @dev Withdrawal: burns shares, executes waterfall, transfers assets.
    function _withdraw(
        address caller,
        address receiver,
        address owner,
        uint256 assets,
        uint256 shares
    ) internal override {
        if (caller != owner) {
            _spendAllowance(owner, caller, shares);
        }

        _burn(owner, shares);

        uint256 totalWithdrawn = _executeWithdrawal(receiver, assets);
        if (totalWithdrawn < assets) revert WithdrawalFailed();
        emit Withdraw(caller, receiver, owner, totalWithdrawn, shares);
    }

    // =========================================================================
    // Admin: Strategy Management
    // =========================================================================

    function addStrategy(IStrategy strategy) external onlyAdmin {
        if (isActiveStrategy[strategy]) revert StrategyAlreadyActive();
        if (strategies.length >= MAX_STRATEGIES) revert TooManyStrategies();
        if (strategy.asset() != asset()) revert StrategyAssetMismatch();

        strategies.push(strategy);
        isActiveStrategy[strategy] = true;

        // Approve strategy to pull assets
        IERC20(asset()).forceApprove(address(strategy), type(uint256).max);

        emit StrategyAdded(address(strategy));
    }

    function removeStrategy(IStrategy strategy) external onlyAdmin nonReentrant {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        _consumeTimelockIfActive(keccak256(abi.encode("removeStrategy", address(strategy))));

        // Withdraw everything first (try-catch: broken strategy cannot block removal)
        uint256 tracked = trackedBalance[strategy];
        uint256 balBefore = IERC20(asset()).balanceOf(address(this));
        try strategy.emergencyWithdraw() {} catch {}
        uint256 recovered = IERC20(asset()).balanceOf(address(this)) - balBefore;
        if (tracked > 0 && recovered < tracked) {
            emit StrategyRemovalFundsLost(address(strategy), tracked - recovered);
        }
        trackedBalance[strategy] = 0;

        // Remove from array (swap-and-pop)
        for (uint256 i = 0; i < strategies.length; i++) {
            if (strategies[i] == strategy) {
                strategies[i] = strategies[strategies.length - 1];
                strategies.pop();
                break;
            }
        }

        isActiveStrategy[strategy] = false;
        targetAllocationBps[strategy] = 0;
        maxAllocationBps[strategy] = 0;
        pausedStrategies[strategy] = false;
        depositFrozenStrategies[strategy] = false;
        IERC20(asset()).forceApprove(address(strategy), 0);

        emit StrategyRemoved(address(strategy));
    }

    function pauseStrategy(IStrategy strategy) external onlyGuardianOrAdmin {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        pausedStrategies[strategy] = true;
        emit StrategyPaused(address(strategy));
    }

    function unpauseStrategy(IStrategy strategy) external onlyAdmin {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        pausedStrategies[strategy] = false;
        emit StrategyUnpaused(address(strategy));
    }

    /// @notice Freeze deposits to a strategy (withdrawals still work).
    ///         Guardian can freeze; only admin can unfreeze.
    function freezeStrategyDeposits(IStrategy strategy) external onlyGuardianOrAdmin {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        if (depositFrozenStrategies[strategy]) revert StrategyAlreadyDepositFrozen();
        depositFrozenStrategies[strategy] = true;
        emit StrategyDepositFrozen(address(strategy));
    }

    function unfreezeStrategyDeposits(IStrategy strategy) external onlyAdmin {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        if (!depositFrozenStrategies[strategy]) revert StrategyNotDepositFrozen();
        depositFrozenStrategies[strategy] = false;
        emit StrategyDepositUnfrozen(address(strategy));
    }

    /// @notice Pull all funds from a strategy back to vault idle and freeze deposits.
    ///         Funds stay as idle — NOT returned to users. Share price unaffected.
    ///         If both withdraw and emergencyWithdraw fail, emits RecallFailed but still
    ///         sets allocation to 0 and freezes deposits to prevent further damage.
    function recallToIdle(IStrategy strategy) external onlyAdmin nonReentrant {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();

        uint256 tracked = trackedBalance[strategy];
        if (tracked > 0) {
            uint256 withdrawn;
            try strategy.withdraw(tracked) returns (uint256 w) {
                withdrawn = w;
            } catch {
                try strategy.emergencyWithdraw() returns (uint256 w) {
                    withdrawn = w;
                } catch {}
            }

            if (withdrawn > 0) {
                if (withdrawn > trackedBalance[strategy]) {
                    trackedBalance[strategy] = 0;
                } else {
                    trackedBalance[strategy] -= withdrawn;
                }
                emit RecalledToIdle(address(strategy), withdrawn);
            } else {
                // Both withdrawal paths failed — funds are stuck in strategy.
                // Still freeze + zero allocation to prevent further deposits.
                emit RecallFailed(address(strategy), tracked);
            }
        }

        targetAllocationBps[strategy] = 0;
        emit AllocationSet(address(strategy), 0);

        if (!depositFrozenStrategies[strategy]) {
            depositFrozenStrategies[strategy] = true;
            emit StrategyDepositFrozen(address(strategy));
        }
    }

    /// @notice Force-redeem all shares of a user, sending assets to the user (not admin).
    ///         When timelockDelay > 0, requires a prior proposeTimelock() with the matching
    ///         operation hash: keccak256(abi.encode("forceRedeem", user)).
    function forceRedeem(address user) external onlyAdmin nonReentrant {
        _consumeTimelockIfActive(keccak256(abi.encode("forceRedeem", user)));
        _accruePerformanceFee();

        uint256 shares = balanceOf(user);
        if (shares == 0) revert NoSharesToRedeem();

        uint256 assets = convertToAssets(shares);
        _burn(user, shares);

        uint256 totalWithdrawn = _executeWithdrawal(user, assets);
        // Allow rounding dust: per-strategy rounding (2 wei each) OR 1 bps of assets,
        // whichever is larger. Some protocols (Morpho share math, Aave ray math) can
        // produce rounding errors beyond 2 wei per strategy.
        uint256 dustTolerance = _dustTolerance(assets);
        if (totalWithdrawn + dustTolerance < assets) revert WithdrawalFailed();
        emit Withdraw(msg.sender, user, user, totalWithdrawn, shares);
        emit ForceRedeemed(user, shares, totalWithdrawn);
    }

    /// @notice Force-redeem multiple users in a single transaction.
    ///         Skips users with 0 shares. Reverts if ALL users have 0 shares.
    ///         When timelockDelay > 0, requires a prior proposeTimelock() with the matching
    ///         operation hash: keccak256(abi.encode("batchForceRedeem", users)).
    function batchForceRedeem(address[] calldata users) external onlyAdmin nonReentrant {
        _consumeTimelockIfActive(keccak256(abi.encode("batchForceRedeem", users)));
        _accruePerformanceFee();

        if (users.length == 0) revert EmptyUserList();
        if (users.length > MAX_FORCE_REDEEM_BATCH) revert BatchTooLarge();

        bool anyRedeemed = false;
        for (uint256 i = 0; i < users.length; i++) {
            uint256 shares = balanceOf(users[i]);
            if (shares == 0) continue;

            uint256 assets = convertToAssets(shares);
            _burn(users[i], shares);

            uint256 totalWithdrawn = _executeWithdrawal(users[i], assets);
            if (totalWithdrawn + _dustTolerance(assets) < assets) revert WithdrawalFailed();
            emit Withdraw(msg.sender, users[i], users[i], totalWithdrawn, shares);
            emit ForceRedeemed(users[i], shares, totalWithdrawn);
            anyRedeemed = true;
        }

        if (!anyRedeemed) revert NoSharesToRedeem();
    }

    // =========================================================================
    // Keeper Operations (admin or keeper)
    // =========================================================================

    function _setAllocation(IStrategy strategy, uint256 bps) internal {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        if (bps > BPS) revert InvalidAllocation();
        uint256 cap = maxAllocationBps[strategy];
        if (cap > 0 && bps > cap) revert AllocationExceedsCap();
        targetAllocationBps[strategy] = bps;
        emit AllocationSet(address(strategy), bps);
    }

    /// @notice Set maximum allocation cap for a strategy (0 = no cap)
    function setMaxAllocation(IStrategy strategy, uint256 maxBps) external onlyAdmin {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        if (maxBps > BPS) revert InvalidAllocation();
        maxAllocationBps[strategy] = maxBps;
        emit MaxAllocationSet(address(strategy), maxBps);
    }

    /// @notice Rebalance: move assets between idle and strategies to match target allocations.
    ///         Skips paused, unhealthy, and broken strategies. Called by keeper or admin.
    ///         Strategy calls are wrapped in try-catch so a single broken strategy cannot
    ///         block the entire rebalance operation.
    function rebalance() external onlyAdminOrKeeper nonReentrant whenNotPaused {
        _rebalance();
    }

    /// @notice Set allocations and rebalance in a single transaction.
    ///         The keeper's primary entry point: pass desired BPS per strategy,
    ///         contract stores them and immediately rebalances to match.
    function rebalance(
        IStrategy[] calldata _strategies,
        uint256[] calldata _bps
    ) external onlyAdminOrKeeper nonReentrant whenNotPaused {
        if (_strategies.length != _bps.length) revert InvalidAllocation();
        for (uint256 i = 0; i < _strategies.length; i++) {
            _setAllocation(_strategies[i], _bps[i]);
        }
        _rebalance();
    }

    /// @dev Revert if the sum of all strategy allocations + idle buffer exceeds 100%.
    function _validateTotalAllocation() internal view {
        uint256 totalBps = idleBufferBps;
        for (uint256 i = 0; i < strategies.length; i++) {
            totalBps += targetAllocationBps[strategies[i]];
        }
        if (totalBps > BPS) revert InvalidAllocation();
    }

    function _rebalance() internal {
        _validateTotalAllocation();
        uint256 total = totalAssets();
        uint256 targetIdle = total.mulDiv(idleBufferBps, BPS);
        uint256 currentIdle = IERC20(asset()).balanceOf(address(this));

        for (uint256 i = 0; i < strategies.length; i++) {
            IStrategy strategy = strategies[i];
            if (pausedStrategies[strategy]) continue;

            // Skip unhealthy strategies -- don't deposit more into them
            // try-catch: broken strategy cannot block rebalance
            try strategy.isHealthy() returns (bool healthy) {
                if (!healthy) continue;
            } catch {
                continue;
            }

            uint256 targetBalance = total.mulDiv(targetAllocationBps[strategy], BPS);

            // Enforce per-strategy cap
            uint256 cap = maxAllocationBps[strategy];
            if (cap > 0) {
                uint256 maxBalance = total.mulDiv(cap, BPS);
                if (targetBalance > maxBalance) {
                    targetBalance = maxBalance;
                }
            }

            uint256 currentBalance = trackedBalance[strategy];

            if (targetBalance > currentBalance && !depositFrozenStrategies[strategy]) {
                // Need to deposit more (skip if deposit-frozen)
                uint256 toDeposit = targetBalance - currentBalance;

                // Respect deviation threshold: only rebalance if deviation exceeds threshold
                if (maxDeviationBps > 0 && toDeposit.mulDiv(BPS, total) < maxDeviationBps) continue;

                uint256 availableForDeposit = currentIdle > targetIdle ? currentIdle - targetIdle : 0;
                if (availableForDeposit == 0) continue;

                toDeposit = toDeposit > availableForDeposit ? availableForDeposit : toDeposit;
                try strategy.deposit(toDeposit) {
                    trackedBalance[strategy] += toDeposit;
                    currentIdle -= toDeposit;
                } catch {
                    continue;
                }
            } else if (currentBalance > targetBalance) {
                // Need to withdraw
                uint256 toWithdraw = currentBalance - targetBalance;

                // Respect deviation threshold
                if (maxDeviationBps > 0 && toWithdraw.mulDiv(BPS, total) < maxDeviationBps) continue;

                try strategy.withdraw(toWithdraw) returns (uint256 withdrawn) {
                    // Safe subtraction: strategy may return slightly more than tracked due to rounding
                    if (withdrawn > trackedBalance[strategy]) {
                        trackedBalance[strategy] = 0;
                    } else {
                        trackedBalance[strategy] -= withdrawn;
                    }
                    currentIdle += withdrawn;
                } catch {
                    continue;
                }
            }
        }

        emit Rebalanced();
    }

    /// @notice Reconcile internal accounting with actual strategy balances.
    ///         Recognizes accrued yield. Called by keeper or admin.
    ///         Broken strategies are skipped (tracked balance unchanged) so a single
    ///         broken strategy cannot block reconciliation of healthy strategies.
    function reconcile() external onlyAdminOrKeeper nonReentrant {
        // Accrue fee at old share price before recognizing new yield.
        // Prevents MEV sandwich: deposit -> reconcile -> withdraw.
        _accruePerformanceFee();

        for (uint256 i = 0; i < strategies.length; i++) {
            IStrategy strategy = strategies[i];
            if (pausedStrategies[strategy]) continue;
            try strategy.balanceOf() returns (uint256 balance) {
                trackedBalance[strategy] = balance;
            } catch {
                // broken strategy — keep stale tracked balance, don't block reconcile
            }
        }
        emit Reconciled();
    }

    /// @notice Harvest rewards from all strategies and send to rewards module.
    ///         No-op if rewardsModule is not set. Broken strategies are skipped.
    function harvestAll() external onlyAdminOrKeeper nonReentrant {
        if (rewardsModule == address(0)) return;

        uint256 totalHarvested = 0;
        for (uint256 i = 0; i < strategies.length; i++) {
            IStrategy strategy = strategies[i];
            if (pausedStrategies[strategy]) continue;
            try strategy.harvest(rewardsModule) returns (uint256 harvested) {
                totalHarvested += harvested;
            } catch {
                // broken strategy — skip, don't block harvest of other strategies
            }
        }

        if (totalHarvested > 0) {
            emit HarvestCompleted(totalHarvested);
        }
    }

    /// @notice Sweep non-asset reward tokens from a strategy to the rewards module.
    ///         Used after RewardsModule.executeClaim() sends merkle-claimed rewards to a strategy.
    /// @param strategy The strategy holding the reward tokens
    /// @param rewardToken The reward token to sweep
    function sweepStrategyReward(
        IStrategy strategy,
        address rewardToken
    ) external onlyAdminOrKeeper nonReentrant {
        if (!isActiveStrategy[strategy]) revert StrategyNotActive();
        if (rewardsModule == address(0)) revert NotRewardsModule();
        uint256 amount = strategy.sweepReward(rewardToken, rewardsModule);
        emit StrategyRewardSwept(address(strategy), rewardToken, rewardsModule, amount);
    }

    /// @notice Collect accrued performance fee (if share price above HWM).
    ///         Called by keeper on schedule or triggered automatically on withdrawals.
    function collectFees() external onlyAdminOrKeeper nonReentrant {
        _accruePerformanceFee();
    }

    // =========================================================================
    // Rewards Module Integration
    // =========================================================================

    /// @notice Entry point for the Rewards Module to deposit swapped rewards back into the vault.
    ///         This enables auto-compounding: rewards are swapped to the base asset off-vault,
    ///         then deposited back here to increase totalAssets for all shareholders.
    /// @param amount Amount of the base asset being deposited as rewards
    function depositRewards(uint256 amount) external nonReentrant {
        if (msg.sender != rewardsModule) revert NotRewardsModule();
        _accruePerformanceFee();
        IERC20(asset()).safeTransferFrom(rewardsModule, address(this), amount);
        emit RewardsDeposited(msg.sender, amount);
    }

    // =========================================================================
    // Admin: Config
    // =========================================================================

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

    function setKeeper(address newKeeper) external onlyAdmin {
        emit KeeperUpdated(keeper, newKeeper);
        keeper = newKeeper; // address(0) = disable keeper, admin-only mode
    }

    function setIdleBuffer(uint256 newBps) external onlyAdmin {
        if (newBps > MAX_IDLE_BUFFER_BPS) revert InvalidBuffer();
        uint256 oldBps = idleBufferBps;
        idleBufferBps = newBps;
        _validateTotalAllocation();
        emit IdleBufferUpdated(oldBps, newBps);
    }

    function setMaxDeviation(uint256 newBps) external onlyAdmin {
        if (newBps > BPS) revert InvalidAllocation();
        emit MaxDeviationUpdated(maxDeviationBps, newBps);
        maxDeviationBps = newBps; // 0 = disabled (rebalance always executes)
    }

    function setRewardsModule(address newModule) external onlyAdmin {
        _consumeTimelockIfActive(keccak256(abi.encode("setRewardsModule", newModule)));
        emit RewardsModuleUpdated(rewardsModule, newModule);
        rewardsModule = newModule;
    }

    function setPerformanceFee(uint256 newBps) external onlyAdmin {
        if (newBps > MAX_PERFORMANCE_FEE_BPS) revert InvalidFee();
        // Fee increases are timelocked so users can exit before the hike
        if (newBps > performanceFeeBps) {
            _consumeTimelockIfActive(keccak256(abi.encode("setPerformanceFee", newBps)));
        }
        // Accrue at old rate before changing so yield isn't retroactively taxed at new rate
        _accruePerformanceFee();
        emit PerformanceFeeUpdated(performanceFeeBps, newBps);
        performanceFeeBps = newBps;
    }

    function setFeeRecipient(address newRecipient) external onlyAdmin {
        if (newRecipient == address(0)) revert ZeroAddress();
        _consumeTimelockIfActive(keccak256(abi.encode("setFeeRecipient", newRecipient)));
        emit FeeRecipientUpdated(feeRecipient, newRecipient);
        feeRecipient = newRecipient;
    }

    function setDepositCap(uint256 newCap) external onlyAdmin {
        emit DepositCapUpdated(depositCap, newCap);
        depositCap = newCap; // 0 = no cap
    }

    function setGuardian(address newGuardian) external onlyAdmin {
        emit GuardianUpdated(guardian, newGuardian);
        guardian = newGuardian; // address(0) = disable guardian
    }

    function pauseVault() external onlyGuardianOrAdmin {
        paused = true;
        emit VaultPaused();
    }

    function unpauseVault() external onlyAdmin {
        paused = false;
        emit VaultUnpaused();
    }

    // =========================================================================
    // Timelock Stub (activated when timelockDelay > 0)
    // =========================================================================

    /// @notice Set the timelock delay. 0 = disabled (immediate execution).
    ///         When enabled, certain admin operations require propose -> wait -> execute.
    ///         Reducing the delay requires a timelocked proposal at the current (longer) delay
    ///         so that users have time to react. Increasing the delay is immediate (more restrictive = safe).
    function setTimelockDelay(uint256 newDelay) external onlyAdmin {
        if (timelockDelay > 0 && newDelay < timelockDelay) {
            _consumeTimelockIfActive(keccak256(abi.encode("setTimelockDelay", newDelay)));
        }
        emit TimelockDelayUpdated(timelockDelay, newDelay);
        timelockDelay = newDelay;
    }

    /// @notice Propose a timelocked operation. Reverts if a pending (non-executed) proposal
    ///         already exists for this hash — cancel it first, then re-propose.
    /// @param operationHash Unique identifier for the operation (e.g., keccak256 of calldata)
    function proposeTimelock(bytes32 operationHash) external onlyAdmin {
        TimelockProposal storage existing = timelockProposals[operationHash];
        if (existing.readyTimestamp > 0 && !existing.executed) revert TimelockAlreadyPending();

        uint256 readyAt = block.timestamp + timelockDelay;
        timelockProposals[operationHash] = TimelockProposal({
            operationHash: operationHash,
            readyTimestamp: readyAt,
            executed: false
        });
        emit TimelockProposed(operationHash, readyAt);
    }

    /// @notice Cancel a pending timelocked operation
    function cancelTimelock(bytes32 operationHash) external onlyAdmin {
        if (timelockProposals[operationHash].readyTimestamp == 0) revert TimelockNotFound();
        delete timelockProposals[operationHash];
        emit TimelockCancelled(operationHash);
    }

    /// @notice Check if a timelocked operation is ready to execute
    function isTimelockReady(bytes32 operationHash) public view returns (bool) {
        TimelockProposal storage proposal = timelockProposals[operationHash];
        return proposal.readyTimestamp > 0
            && block.timestamp >= proposal.readyTimestamp
            && !proposal.executed;
    }

    // =========================================================================
    // View Helpers
    // =========================================================================

    function strategiesCount() external view returns (uint256) {
        return strategies.length;
    }

    function getStrategies() external view returns (IStrategy[] memory) {
        return strategies;
    }

    function idleBalance() external view returns (uint256) {
        return IERC20(asset()).balanceOf(address(this));
    }

    /// @notice Check if any strategy deviates from target allocation beyond maxDeviationBps
    function needsRebalance() external view returns (bool) {
        if (maxDeviationBps == 0) return false;
        uint256 total = totalAssets();
        if (total == 0) return false;

        for (uint256 i = 0; i < strategies.length; i++) {
            IStrategy strategy = strategies[i];
            if (pausedStrategies[strategy]) continue;

            uint256 targetBalance = total.mulDiv(targetAllocationBps[strategy], BPS);
            uint256 currentBalance = trackedBalance[strategy];
            uint256 deviation = targetBalance > currentBalance
                ? targetBalance - currentBalance
                : currentBalance - targetBalance;

            if (deviation.mulDiv(BPS, total) >= maxDeviationBps) return true;
        }
        return false;
    }

    /// @notice Get allocation status for all strategies
    function getAllocationStatus()
        external
        view
        returns (IStrategy[] memory strats, uint256[] memory targets, uint256[] memory actuals, bool[] memory healthy)
    {
        uint256 len = strategies.length;
        strats = strategies;
        targets = new uint256[](len);
        actuals = new uint256[](len);
        healthy = new bool[](len);

        for (uint256 i = 0; i < len; i++) {
            targets[i] = targetAllocationBps[strategies[i]];
            actuals[i] = trackedBalance[strategies[i]];
            try strategies[i].isHealthy() returns (bool h) {
                healthy[i] = h;
            } catch {
                healthy[i] = false;
            }
        }
    }

    // =========================================================================
    // Internal
    // =========================================================================

    /// @dev Dust tolerance for forceRedeem: per-strategy rounding (2 wei each) OR 1 bps
    ///      of the asset amount, whichever is larger. Covers protocols with larger rounding
    ///      errors (Morpho share math, Aave ray math).
    function _dustTolerance(uint256 assets) internal view returns (uint256) {
        uint256 perStrategy = strategies.length * 2;
        uint256 bpsBased = assets / BPS; // 1 bps
        return perStrategy > bpsBased ? perStrategy : bpsBased;
    }

    /// @dev Enforce timelock when active. No-op when timelockDelay == 0.
    ///      Marks the proposal as executed so it cannot be replayed.
    function _consumeTimelockIfActive(bytes32 opHash) internal {
        if (timelockDelay == 0) return;

        TimelockProposal storage proposal = timelockProposals[opHash];
        if (proposal.readyTimestamp == 0) revert TimelockNotFound();
        if (block.timestamp < proposal.readyTimestamp) revert TimelockNotReady();
        if (proposal.executed) revert TimelockAlreadyExecuted();
        proposal.executed = true;
    }

    /// @dev Withdrawal waterfall: idle -> strategies (skip paused/broken) -> revert if insufficient.
    ///      Strategy calls are wrapped in try-catch so a single broken strategy cannot block
    ///      all user withdrawals. Broken strategies are silently skipped.
    function _executeWithdrawal(address receiver, uint256 assets) internal returns (uint256 totalWithdrawn) {
        uint256 idle = IERC20(asset()).balanceOf(address(this));
        uint256 remaining = assets;

        // Step 1: Use idle buffer
        if (idle >= remaining) {
            IERC20(asset()).safeTransfer(receiver, remaining);
            return remaining;
        }

        if (idle > 0) {
            remaining -= idle;
        }

        // Step 2: Pull from strategies (skip paused and broken)
        for (uint256 i = 0; i < strategies.length && remaining > 0; i++) {
            IStrategy strategy = strategies[i];
            if (pausedStrategies[strategy]) continue;

            // try-catch: broken strategy cannot block withdrawals
            try strategy.availableLiquidity() returns (uint256 available) {
                if (available == 0) continue;

                uint256 toWithdraw = remaining > available ? available : remaining;
                try strategy.withdraw(toWithdraw) returns (uint256 withdrawn) {
                    if (withdrawn > trackedBalance[strategy]) {
                        trackedBalance[strategy] = 0;
                    } else {
                        trackedBalance[strategy] -= withdrawn;
                    }

                    // Saturating subtraction: strategy may return slightly more than requested due to rounding
                    remaining = withdrawn >= remaining ? 0 : remaining - withdrawn;
                } catch {
                    continue;
                }
            } catch {
                continue;
            }
        }

        totalWithdrawn = assets - remaining;

        IERC20(asset()).safeTransfer(receiver, totalWithdrawn);
    }

    /// @dev Total liquidity available for withdrawal: idle + sum of strategy available liquidity.
    ///      Uses try-catch so a broken strategy cannot make maxWithdraw/maxRedeem revert.
    ///      Each strategy's available is reduced by 1 to account for rounding in underlying
    ///      protocol withdrawals, ensuring maxWithdraw/maxRedeem never over-promise
    ///      (ERC-4626 compliance: `redeem(maxRedeem(x))` must not revert).
    function _availableLiquidity() internal view returns (uint256 available) {
        available = IERC20(asset()).balanceOf(address(this));
        for (uint256 i = 0; i < strategies.length; i++) {
            IStrategy strategy = strategies[i];
            if (pausedStrategies[strategy]) continue;
            try strategy.availableLiquidity() returns (uint256 stratAvail) {
                available += stratAvail > 2 ? stratAvail - 2 : 0;
            } catch {
                // broken strategy — skip, don't block view functions
            }
        }
    }

    /// @dev Simulate the number of fee shares that would be minted if performance fee
    ///      is accrued now. Used by maxWithdraw/maxRedeem to account for fee dilution
    ///      and maintain ERC-4626 compliance (withdraw(maxWithdraw(x)) must not revert).
    function _pendingFeeShares() internal view returns (uint256) {
        if (performanceFeeBps == 0) return 0;

        uint256 oneShare = 10 ** decimals();
        uint256 currentSharePrice = convertToAssets(oneShare);
        if (currentSharePrice <= highWaterMark) return 0;

        uint256 gain = currentSharePrice - highWaterMark;
        uint256 totalShares = totalSupply();
        uint256 fee = gain.mulDiv(totalShares, oneShare).mulDiv(performanceFeeBps, BPS);

        if (fee == 0) return 0;
        return convertToShares(fee);
    }

    /// @dev Accrue HWM-based performance fee. Mints fee shares to feeRecipient
    ///      when share price exceeds the high-water mark. Returns fee in asset terms.
    ///      Called on every withdrawal (dodge-proof) and by collectFees()/setPerformanceFee().
    ///      HWM is set to the POST-mint share price so that future yield is measured from
    ///      the actual per-share value after dilution. This prevents an untaxed "gap" between
    ///      the post-dilution price and the old (pre-mint) HWM.
    function _accruePerformanceFee() internal returns (uint256 fee) {
        uint256 oneShare = 10 ** decimals();
        uint256 currentSharePrice = convertToAssets(oneShare);
        if (currentSharePrice <= highWaterMark) return 0;

        uint256 gain = currentSharePrice - highWaterMark;

        if (performanceFeeBps == 0) {
            // Always advance HWM — prevents retroactive fee when re-enabling from zero
            highWaterMark = currentSharePrice;
            return 0;
        }

        uint256 totalShares = totalSupply();
        fee = gain.mulDiv(totalShares, oneShare).mulDiv(performanceFeeBps, BPS);

        if (fee > 0) {
            uint256 feeShares = convertToShares(fee);
            if (feeShares > 0) {
                _mint(feeRecipient, feeShares);
                emit PerformanceFeeCollected(fee, feeRecipient);
            }
        }

        // Post-mint HWM: reflects actual share price after fee dilution.
        // If no shares were minted (fee rounded to 0), equals currentSharePrice.
        highWaterMark = convertToAssets(oneShare);
    }
}
