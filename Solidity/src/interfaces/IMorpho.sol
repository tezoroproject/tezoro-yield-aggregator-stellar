// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

/// @notice MarketParams identifies a unique Morpho Blue market
struct MarketParams {
    address loanToken;
    address collateralToken;
    address oracle;
    address irm;
    uint256 lltv;
}

/// @notice On-chain state of a Morpho Blue market
struct MorphoMarket {
    uint128 totalSupplyAssets;
    uint128 totalSupplyShares;
    uint128 totalBorrowAssets;
    uint128 totalBorrowShares;
    uint128 lastUpdate;
    uint128 fee;
}

/// @notice Per-user position in a Morpho Blue market
struct Position {
    uint128 supplyShares;
    uint128 borrowShares;
    uint128 collateral;
}

type Id is bytes32;

/// @notice Minimal Morpho Blue interface -- only the functions we use
interface IMorpho {
    function supply(
        MarketParams memory marketParams,
        uint256 assets,
        uint256 shares,
        address onBehalf,
        bytes calldata data
    ) external returns (uint256 assetsSupplied, uint256 sharesSupplied);

    function withdraw(
        MarketParams memory marketParams,
        uint256 assets,
        uint256 shares,
        address onBehalf,
        address receiver
    ) external returns (uint256 assetsWithdrawn, uint256 sharesWithdrawn);

    function position(Id id, address user) external view returns (Position memory);

    function market(Id id) external view returns (MorphoMarket memory);

    function idToMarketParams(Id id) external view returns (MarketParams memory);

    function accrueInterest(MarketParams memory marketParams) external;
}
