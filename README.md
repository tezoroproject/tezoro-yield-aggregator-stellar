# Tezoro Yield Aggregator - Stellar

> **Status: Draft / Work in Progress**

Soroban smart contracts for the Tezoro yield aggregator on Stellar. Accepts USDC deposits, mints yield-bearing tUSDC shares, and distributes capital across lending strategies (starting with Blend v2).

## Architecture

```
User (USDC) --> TezoroVault --> Strategy adapters --> DeFi protocols
                  |                                      |
                  |-- idle buffer (3%)                   |-- Blend v2
                  |-- share accounting (tUSDC)           |-- (future: Aquarius, Soroswap)
                  |-- cross-chain bridged balance
                  |
              Keeper (off-chain)
                  |-- rebalancing
                  |-- yield harvesting
                  |-- cross-chain attestation
```

### Contracts

| Contract | Description |
|----------|-------------|
| **vault** | Core vault implementing SEP-41 token interface. Handles deposits, redemptions, share accounting with virtual offset (inflation attack protection), strategy management, and cross-chain balance attestation. |
| **blend-strategy** | Strategy adapter for Blend v2 lending protocol. Deposits USDC as collateral via `submit(SupplyCollateral)`, monitors pool utilization and backstop health. |
| **mock-strategy** | Test-only strategy that simulates yield accrual. Used for vault integration testing without external protocol dependencies. |

### Roles

- **Admin** -- configuration, strategy management, unpause
- **Guardian** -- emergency pause (no fund access)
- **Keeper** -- rebalancing, tracked balance updates, bridged balance attestation (cannot extract user funds)
- **User** -- deposit, withdraw

### Key Design Decisions

- **Virtual shares offset** (10^6) prevents first-depositor inflation attacks
- **Internal tracked balance** per strategy prevents donation attacks
- **Bridged balance attestation** -- keeper reports EVM-side capital so share price reflects total cross-chain AUM
- **Idle buffer** -- configurable % of TVL held uninvested for instant small withdrawals
- **Withdrawals always open** -- users can exit at any time; vault never locks funds

## Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- Soroban CLI: `cargo install --locked soroban-cli`
- Wasm target: `rustup target add wasm32-unknown-unknown`

## Build

```bash
cargo build --target wasm32-unknown-unknown --release
```

## Test

```bash
cargo test
```

## Project Structure

```
contracts/
  vault/src/lib.rs            -- core vault (SEP-41, deposit/redeem, strategy mgmt, keeper ops)
  blend-strategy/src/lib.rs   -- Blend v2 adapter (SupplyCollateral / WithdrawCollateral)
  mock-strategy/src/lib.rs    -- test mock with simulated yield
```
