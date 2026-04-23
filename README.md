# Tezoro Yield Aggregator — Stellar

Production-grade yield aggregator on Stellar / Soroban. An ERC-4626-style vault accepts USDC, pools the deposits, and routes them into the Blend Protocol v2 lending pool through a pluggable strategy interface. An off-chain keeper allocates surplus idle balance into Blend every ~20 s; user redemptions use a single-transaction withdrawal waterfall (idle → strategies) so exits never depend on a prior keeper deallocation.

This repository contains the Soroban contracts, integration tests, and the Stellar allocation keeper. Built as the Tranche 1 deliverable of the Stellar Community Fund grant awarded to Tezoro.

---

## Live deployments (testnet)

| Contract | Address |
|---|---|
| TezoroVault | [`CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5`](https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5) |
| BlendStrategy | [`CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6`](https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6) |
| Asset (Blend testnet USDC) | `CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU` |
| Underlying pool (Blend Testnet V2) | [`CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF`](https://stellar.expert/explorer/testnet/contract/CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF) |

Share token: `Tezoro USDC-A` / `tUSDC-A`. Performance fee: 15 %. Idle buffer: 3 %. Upgrade timelock: 48 h default, 1 h minimum.

**Live yield API** (all four Blend Testnet V2 reserves):

```bash
curl -s https://www.tezoro.io/api/agg/api/stellar/markets | jq
```

Full address list and per-role identities are in [`docs/tranche-1/addresses.md`](docs/tranche-1/addresses.md).

---

## Repository layout

```
contracts/                      # Cargo workspace — Soroban contracts & harnesses
├── Cargo.toml
├── rust-toolchain.toml         # pinned to Rust 1.94.0 + wasm32 + llvm-tools
├── tarpaulin.toml              # coverage config
├── contracts/
│   ├── tezoro-common/          # Shared StrategyInterface trait + errors + TTL
│   ├── tezoro-vault/           # ERC-4626-style vault (deposit / redeem / allocate / waterfall)
│   ├── blend-strategy/         # Blend v2 adapter (submit SupplyCollateral / WithdrawCollateral)
│   └── mock-strategy/          # Test-double strategy used in vault unit tests
├── tests/
│   └── mainnet-fork/           # End-to-end tests against a forked mainnet Blend pool
└── scripts/
    └── coverage.sh             # Branch-coverage runner (nightly + cargo-llvm-cov)

# The mainnet-fork harness was extracted into its own open-source crate.
# Source: https://github.com/lobotomoe/soroban-fork
# Registry: https://crates.io/crates/soroban-fork

keeper/                         # @tezoro/stellar-keeper — standalone allocation keeper (TypeScript)
├── src/
└── package.json

docs/tranche-1/                 # SCF Tranche-1 submission bundle
├── addresses.md                # Current deployment addresses
├── completion-report.md        # Full deliverable breakdown
├── submission.md               # Ready-to-paste SCF form answers
└── api-stellar-markets.json    # Snapshot of the live yield API response
```

---

## Design notes

### Withdrawal waterfall

`redeem(shares)` is a single transaction regardless of how the user's position is split between idle USDC and the Blend strategy. The vault serves from idle first; any shortfall is pulled from registered strategies in order via `strategy.withdraw()`. Each hop is capped by `strategy.available_liquidity()` so the vault never asks for more than the underlying pool can actually deliver on the current ledger — which matters on a Blend pool at high utilization, where the strategy's `tracked_balance` can legitimately exceed the pool's free liquidity.

### `available_liquidity()`

Returns the amount the strategy can deliver *right now*, in underlying asset units:

```
pool_supplied  = b_supply * b_rate / 1e12     // Blend's b_token scaling
pool_borrowed  = d_supply * d_rate / 1e12
pool_available = pool_supplied - pool_borrowed
available      = min(tracked_balance, pool_available) - rounding_margin
```

The 2-stroop rounding margin mirrors the EVM vault's `_availableLiquidity() - 2` and exists for the same reason: the Blend pool rounds *up* when converting a requested underlying amount back to b_tokens for burn, so an exact-match quote-then-request race can require 1–2 more b_tokens than the strategy holds. 2 stroops is 2×10⁻⁷ USDC — below any display resolution — but eliminates the race.

### Healthcheck gate on `allocate()`

Before deploying funds, the vault calls `strategy.is_healthy()`. The Blend strategy returns `false` when the pool's utilization exceeds the configured ceiling, when backstop coverage falls below the configured floor, or when the pool is unreachable (the conservative default). If health returns false, the vault rejects the allocation with `VaultError::StrategyUnhealthy`. The off-chain keeper mirrors this pre-check before submitting, so an unhealthy pool never burns keeper gas on a doomed transaction.

### Pre-transfer deposit pattern

The vault transfers the underlying asset *into* the strategy contract *before* calling `strategy.deposit(...)`. This eliminates the cross-contract allowance dance — the strategy never needs to pull tokens from the vault, it just deploys tokens it already holds. Symmetrically, `strategy.withdraw()` transfers tokens back to the caller, so the vault's ownership of redeemed funds is explicit at the token-layer.

### Typed strategy interface

`StrategyInterface` is declared in `tezoro-common` with `Result<T, StrategyError>` returns. The `#[contractclient]` macro generates the client bindings the vault uses to cross-call any contract that implements the interface — Blend today, others (Fluid, Yieldblox, Soroswap LP, …) on the same interface tomorrow.

---

## Building and testing

### Prerequisites

- Rust 1.94.0 (pinned in `contracts/rust-toolchain.toml`)
- `wasm32-unknown-unknown` target + `rustfmt`, `clippy`, `llvm-tools-preview` components (auto-installed by the toolchain file)
- Optional: [`stellar` CLI](https://developers.stellar.org/docs/tools/developer-tools/cli/install-cli) for WASM-level optimization + deployment
- Optional: `pnpm` and Node.js 24 for the keeper

### Contracts

```bash
cd contracts
cargo fmt --all -- --check
cargo clippy --workspace --exclude mainnet-fork-tests --all-targets --locked -- -D warnings
cargo test    --workspace --exclude mainnet-fork-tests --locked
```

Runs **188 tests** across the vault, the Blend strategy (unit + integration + security + coverage-gaps), the mock strategy, and the in-tree fork harness. Completes in under a minute on a recent laptop.

### Mainnet-fork integration tests

The mainnet-fork suite deploys our compiled WASM onto a forked mainnet ledger against the real Blend pool + USDC contracts. Requires a prior optimization pass (the tests `include_bytes!` `*.optimized.wasm` from `target/`):

```bash
cd contracts
cargo build --target wasm32-unknown-unknown --release -p tezoro-vault -p blend-strategy
stellar contract optimize --wasm target/wasm32-unknown-unknown/release/tezoro_vault.wasm
stellar contract optimize --wasm target/wasm32-unknown-unknown/release/blend_strategy.wasm
cargo test -p mainnet-fork-tests --locked
```

Adds 6 more tests (mainnet share accounting, NAV with real prices, vault+strategy+Blend end-to-end, upgrade timelock with ledger warp).

### Coverage

```bash
cd contracts
cargo install --locked cargo-tarpaulin
cargo tarpaulin              # ~98 % line coverage on contract code
# HTML report at coverage/tarpaulin-report.html
```

### Keeper

```bash
cd keeper
pnpm install
pnpm typecheck
# Set STELLAR_KEEPER_SECRET in .env, then:
pnpm dev
```

---

## License

**Unlicensed — all rights reserved.** The source is published for transparency and grant-review purposes only. No license to use, copy, modify, or distribute is granted. Each crate in `contracts/` is marked `license = "UNLICENSED"` in its `Cargo.toml`, and each Soroban contract's `lib.rs` carries a `// SPDX-License-Identifier: UNLICENSED` header. A permissive open-source license may be applied in a future release.

---

## Status

Tranche 1 of the Stellar Community Fund grant. See [`docs/tranche-1/completion-report.md`](docs/tranche-1/completion-report.md) for the full deliverable breakdown.
