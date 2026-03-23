# Tezoro Yield Aggregator

Smart contracts for the Tezoro yield aggregator. Accepts USDC deposits, mints yield-bearing tUSDC shares, and distributes capital across lending strategies via a keeper-managed rebalancing system.

## Repository Structure

```
Soroban/    -- Soroban contracts for Stellar (work in progress).
Solidity/   -- Production EVM contracts (Foundry). Deployed on Ethereum, Base, Polygon, BSC, Optimism.
```

## Soroban (Stellar)

> [!CAUTION]
> **WORK IN PROGRESS** -- Soroban contracts are under active development and NOT yet deployed. Interfaces and storage layout may change without notice.

SEP-41 compatible vault for Stellar. Starting with Blend v2 strategy.

| Contract | Description |
|----------|-------------|
| **vault** | Core vault -- SEP-41 token interface, deposit/redeem, share accounting with virtual offset, strategy management, cross-chain balance attestation. |
| **blend-strategy** | Blend v2 lending adapter (SupplyCollateral / WithdrawCollateral). |
| **mock-strategy** | Test mock with simulated yield. |

### Key Design Decisions

- **Virtual shares offset** (10^6) prevents first-depositor inflation attacks
- **Internal tracked balance** per strategy prevents donation attacks
- **Bridged balance attestation** -- keeper reports EVM-side capital so share price reflects total cross-chain AUM
- **Idle buffer** -- configurable % of TVL held uninvested for instant small withdrawals

## Solidity (EVM)

ERC-4626 vault with multi-strategy architecture. Supports Aave V3, Compound V3, Morpho Blue, Fluid, and generic ERC-4626 strategies.

| Contract | Description |
|----------|-------------|
| **TezoroV1_1** | Core vault -- ERC-4626 share accounting, strategy management, performance fees, keeper-driven allocation. |
| **RewardsModule** | Claims and swaps protocol reward tokens (COMP, MORPHO, etc.) back into the vault. |
| **AaveV3Strategy** | Aave V3 lending adapter. |
| **CompoundV3Strategy** | Compound V3 (Comet) adapter. |
| **MorphoBlueMultiStrategy** | Morpho Blue multi-market adapter with per-market allocation. |
| **ERC4626MultiStrategy** | Generic adapter for any ERC-4626 vault (Fluid, MetaMorpho, etc.). |
| **FluidStrategy** | Fluid lending adapter. |

## Roles

- **Admin** -- configuration, strategy management, unpause
- **Guardian** -- emergency pause (no fund access)
- **Keeper** -- rebalancing, allocation, reward harvesting (cannot extract user funds)
- **User** -- deposit, withdraw
