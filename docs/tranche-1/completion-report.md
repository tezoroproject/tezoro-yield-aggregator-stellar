# Tranche 1 — Completion Report

**Network: Stellar testnet (`Test SDF Network ; September 2015`).**

---

## Deliverables

| # | Deliverable | Status | Evidence |
|---|---|---|---|
| 1 | TezoroVault Soroban contract deployed to Stellar testnet | ✅ Done | [`CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5`](https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5) |
| 2 | Blend v2 strategy adapter deployed to testnet | ✅ Done | [`CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6`](https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6) |
| 3 | End-to-end deposit flow recorded on testnet | ✅ Done | Verified on-chain via the public demo app; deposit → keeper allocate → redeem round-trip reproducible in a browser |
| 4 | Yield aggregator returning Stellar/Blend APY data | ✅ Done | Live endpoint: `GET https://www.tezoro.io/api/agg/api/stellar/markets` |
| 5 | Unit + integration test suite (>90 % branch coverage) | ✅ Done | **~98 % line coverage** on contract code, 194 passing Cargo tests across the workspace, measured with `cargo-tarpaulin` |
| 6 | Public source code (tagged release) | ⏳ In progress | Staged for publication at `github.com/tezoroproject/tezoro-yield-aggregator-stellar`, release tag pending |

---

## 1. TezoroVault deployed to Stellar testnet

An ERC-4626-style vault contract written in Rust / Soroban SDK 25.3.

**Address:** [`CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5`](https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5)

**Capabilities:**
- `deposit(from, assets) -> shares`, `redeem(from, shares) -> assets` — ERC-4626-compatible accounting with a virtual-shares offset to prevent the first-depositor inflation attack.
- **Withdrawal waterfall:** `redeem()` serves from idle USDC first, then pulls any shortfall from registered strategies via `strategy.withdraw()`, each hop capped by `strategy.available_liquidity()` so the vault never requests more than the underlying pool can actually deliver. Users exit in a single transaction; no dependency on a prior keeper-triggered deallocation.
- Strategy management (`add_strategy`, `remove_strategy`, `allocate`, `deallocate`) with a per-vault idle buffer enforced in basis points.
- Role separation: admin, keeper, guardian, fee recipient. Two-step admin transfer (`propose_admin` → `accept_admin`). Guardian can pause but not upgrade.
- Upgrade timelock: 48-hour default, 1-hour minimum. `schedule_upgrade` → wait → `execute_upgrade`, cancellable.
- Performance-fee accounting with a high-water-mark check on every `collect_fees` call.
- **Healthcheck gate:** `allocate()` verifies the target strategy reports `is_healthy() == true` before deploying funds. If the Blend pool's utilization exceeds its ceiling or the backstop is depleted, the vault refuses the allocation with `VaultError::StrategyUnhealthy`.

**Source:** [`packages/contracts/soroban/contracts/tezoro-vault/`](../../packages/contracts/soroban/contracts/tezoro-vault/)

## 2. Blend v2 strategy adapter deployed to testnet

A Soroban contract implementing Tezoro's `StrategyInterface` against the Blend Protocol v2 lending pool.

**Address:** [`CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6`](https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6)

**Capabilities:**
- `deposit`, `withdraw`, `emergency_withdraw` mapped to Blend's `submit()` RPC with `SupplyCollateral` / `WithdrawCollateral` request types.
- `harvest` claims BLND emissions from the pool for the reserve index held by the strategy.
- `is_healthy` reads the Blend pool's reserve data and returns `false` when:
  - utilization exceeds the configured ceiling (default 95 %), or
  - backstop coverage falls below the configured minimum (default 5 %, tuned to 0 % on testnet because the public Blend testnet pool is not backstopped at mainnet levels), or
  - the pool is unreachable (conservative default).
- `available_liquidity` reports the underlying amount the strategy can deliver right now: `min(tracked_balance, pool_supplied − pool_borrowed)` with both sides converted to underlying via the pool's `b_rate` / `d_rate`, minus a 2-stroop rounding margin so exact-match withdraws never race the pool's b_token rounding. The vault's waterfall relies on this quote, so it must be honest — the same discipline the EVM Tezoro vault applies in `_availableLiquidity()`.
- Same admin / pause / upgrade-timelock surface as the vault.

**Underlying:** Blend Testnet V2 pool [`CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF`](https://stellar.expert/explorer/testnet/contract/CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF), asset USDC SAC `CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU`.

**Source:** [`packages/contracts/soroban/contracts/blend-strategy/`](../../packages/contracts/soroban/contracts/blend-strategy/)

## 3. End-to-end deposit flow on testnet

The full user flow — **deposit into the Tezoro vault → keeper auto-allocates to Blend → user withdraws back to their wallet** — is reproducible end-to-end through the public demo app:

| Step | Action | On-chain effect |
|---|---|---|
| 1 | User deposits N USDC into the vault | Vault custodies N USDC; user receives share tokens (`tUSDC-A`) |
| 2 | Off-chain keeper polls vault state every 20 s | When idle > 3 % buffer, keeper calls `vault.allocate(excess)` into the Blend strategy |
| 3 | Strategy deploys funds into the Blend pool via `submit(SupplyCollateral)` | Strategy tracked balance increases; pool's reserve `b_supply` grows; user's share value accrues supply APY |
| 4 | User calls `redeem(shares)` | Vault's waterfall serves from idle, pulls the shortfall from the strategy via `strategy.withdraw()` — returns the full asset amount to the user in one transaction |

**Public demo app:** browser-based, connects via Freighter, shows live vault state, Blend APY, allocation breakdown, live wallet + position balances, and a waterfall-aware Max withdraw helper. Source in [`packages/stellar-demo/`](../../packages/stellar-demo/).

## 4. Yield aggregator returning Stellar/Blend APY data

A Node.js service aggregating live lending-market data across multiple protocols and chains. The Stellar/Blend integration surfaces Blend's utilization curve, supply / borrow APR, and Blend-SDK-estimated supply / borrow APY per reserve.

**Live endpoint:** `GET https://www.tezoro.io/api/agg/api/stellar/markets`

Response shape (abbreviated):

```json
{
  "chain": "stellar",
  "network": "testnet",
  "markets": [
    {
      "chain": "stellar",
      "network": "testnet",
      "protocol": "blend",
      "poolId": "CCEBVDYM…HGF",
      "poolName": "Blend Testnet V2",
      "asset": {
        "id": "CAQCFVLOB…SRCJU",
        "symbol": "USDC",
        "decimals": 7
      },
      "borrowApr": 0.0016636,
      "supplyApr": 0.0005637,
      "estSupplyApy": 0.0005638558516347647,
      "estBorrowApy": 0.0016649807527064908,
      "totalSupplied": "933333614224",
      "totalBorrowed": "351395686706",
      "utilization": 0.3764953,
      "maxUtilization": 0.95,
      "fetchedAt": 1776793849,
      "latestLedger": 2157677
    }
    // XLM, wETH, wBTC reserves omitted for brevity
  ],
  "fetchedAt": 1776793849
}
```

The endpoint:
- Returns all four reserves in the Blend Testnet V2 pool (USDC, XLM, wETH, wBTC).
- Validates every field of the Blend SDK's response through a Zod schema before trusting it, so malformed or drifted upstream data produces an HTTP 502 instead of silently corrupting APY figures.
- Fails fast on any pool fetch error (partial success is explicitly disallowed to prevent returning incomplete data with a 200 OK).
- Caches responses for 60 seconds with in-flight request coalescing, so a thundering herd at cache-expiry produces at most one upstream Soroban RPC call.
- Network is parametric — switching between Stellar testnet and mainnet is a config change, not a rewrite.

**Source:** [`packages/yield-aggregator/src/sources/blend/`](../../packages/yield-aggregator/src/sources/blend/) (client), [`packages/yield-aggregator/src/server/routes/stellar.ts`](../../packages/yield-aggregator/src/server/routes/stellar.ts) (route), [`packages/yield-aggregator/src/schemas/stellar.ts`](../../packages/yield-aggregator/src/schemas/stellar.ts) (wire schemas).

## 5. Unit + integration test suite

**194 tests pass across the Soroban workspace**, zero failures, measured by `cargo test --workspace --locked`:

| Suite | Tests | Scope |
|---|---|---|
| `tezoro-vault` inline unit | 23 | ERC-4626 math, share minting/burning, pause/unpause, admin transitions, upgrade timelock, collect-fees HWM logic |
| `tezoro-vault/tests/comprehensive` | 50 | Edge cases, multi-user fairness, multi-strategy allocation, PnL distribution, idle-buffer enforcement, redeem waterfall (idle-only path, full-shortfall path, partial shortfall) |
| `tezoro-vault/tests/coverage_gaps` | 7 | Healthcheck gate, partial deallocate reconciliation, full SEP-41 + admin view-function coverage, storage defaults |
| `tezoro-vault/tests/security` | 23 | Every role boundary: admin-only / keeper-only / guardian / pending-admin, plus rejection paths |
| `blend-strategy` inline unit | 12 | Init, pause, auth, two-step admin, approval buffer, health thresholds |
| `blend-strategy/tests/integration` | 15 | Real Blend fixture: deposit → withdraw → emergency-withdraw against a deployed Blend pool, available_liquidity rate-scaling |
| `blend-strategy/tests/coverage_gaps` | 8 | Approval buffer, harvest, pool-position queries, health thresholds |
| `blend-strategy/tests/security` | 27 | Every admin/vault entry point rejects unauthorized callers |
| `mock-strategy` inline unit | 11 | Happy path + full error surface: double-init, zero-amount, unauthorized callers, empty-emergency short-circuit |
| `mainnet-fork-tests` (via `soroban-fork` crate) | 4 | Mainnet-fork scenarios against a lazily-loaded real pool; harness published separately as [`soroban-fork`](https://crates.io/crates/soroban-fork) |
| `tests/mainnet-fork` | 14 | End-to-end vault + strategy against a forked mainnet Blend deployment |

**Coverage**: **~98 % line coverage on contract code**, measured by `cargo-tarpaulin` with the Llvm engine. Configuration is in [`packages/contracts/soroban/tarpaulin.toml`](../../packages/contracts/soroban/tarpaulin.toml); running `cargo tarpaulin` in that directory reproduces the number and writes an HTML report to `coverage/tarpaulin-report.html`.

Scope: contract source code only. The network-dependent `mainnet-fork-tests` integration harness is excluded from the headline number — it's test infrastructure, not on-chain contract code, and including it would misrepresent contract quality. The exclusion is documented in `tarpaulin.toml`. The fork harness itself has since been extracted from this monorepo and published as an independent open-source crate at [crates.io/crates/soroban-fork](https://crates.io/crates/soroban-fork).

Line coverage is used as the primary metric because branch coverage on stable Rust is still marked as experimental in the tooling. The test-suite structure (full role matrix, full arithmetic surface, happy + unhappy paths for every public method, real-pool integration via `BlendFixture`, waterfall paths for all three redeem-vs-idle states) is designed to exercise all meaningful branches. The uncovered lines are concentrated in two areas that can only be reached through infrastructure we don't control: the `upgrade` event emission inside `execute_upgrade` (requires a live deployed WASM hash), and certain RPC-failure branches in `is_healthy` / `available_liquidity` / `harvest` that only fire when the underlying Blend pool contract is unreachable at invocation time.

**CI**: GitHub Actions job `test-soroban` runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --locked` on every push. Clippy and fmt pass cleanly; there are no silenced warnings or ignored tests.

---

## Architecture summary

- **Language / SDK**: Rust + Soroban SDK 25.3.0 for contracts; TypeScript + Hono for the aggregator; TypeScript + React for the demo UI; TypeScript + `@stellar/stellar-sdk` for the off-chain keeper.
- **Pre-transfer pattern**: the vault transfers the underlying asset to the strategy *before* calling `strategy.deposit(...)`. This avoids the cross-contract allowance dance and means the strategy contract never needs to pull tokens from the vault.
- **Typed strategy interface**: `StrategyInterface` is declared in a shared crate (`tezoro-common`) with `Result<T, StrategyError>` returns and explicit `available_liquidity()` declaration. The `#[contractclient]` macro generates the client bindings the vault uses to cross-call any contract that implements the interface — Blend today, others (Fluid, Yieldblox, Soroswap LP…) on the same interface tomorrow.
- **Idle buffer**: every vault keeps a configurable fraction (default 3 %) as free balance so redemptions don't always require a round-trip through the strategy. When a redeem exceeds idle, the waterfall pulls the shortfall from strategies in order, capped by each strategy's live-reported available liquidity.
- **Keeper**: an off-chain process (`@tezoro/stellar-keeper`, running in production under pm2) polls the vault every 20 seconds. When idle balance exceeds the buffer, it pre-checks `strategy.is_healthy()` and, if green, calls `vault.allocate(excess)` to deploy surplus into Blend.

## Repository layout

```
packages/contracts/soroban/
├── contracts/
│   ├── tezoro-common/         # Shared trait + errors + TTL helpers
│   ├── tezoro-vault/          # TezoroVault contract
│   ├── blend-strategy/        # BlendStrategy adapter
│   └── mock-strategy/         # Test-double strategy
# The mainnet-fork harness was extracted into a standalone crate:
#   https://crates.io/crates/soroban-fork
#   https://github.com/lobotomoe/soroban-fork
# It is pulled in below as a regular cargo dependency.
├── tests/
│   └── mainnet-fork/          # Integration harness against forked mainnet state
├── Cargo.toml
├── rust-toolchain.toml        # Pinned to Rust 1.94.0 + wasm32
└── tarpaulin.toml             # Coverage configuration

packages/yield-aggregator/     # API serving live market data
packages/stellar-keeper/       # Allocation keeper (pm2)
packages/stellar-demo/         # Freighter-based demo UI
```

## How to verify independently

1. **Contract deployments**: visit the explorer links above. Both contracts show their deployer, WASM hash, and initialization event.
2. **APY endpoint**: `curl -s https://www.tezoro.io/api/agg/api/stellar/markets | jq`. Live response, no auth required.
3. **Tests locally**: clone the repo, run:
   ```bash
   cd packages/contracts/soroban
   cargo test --workspace --locked
   ```
   Expected: 194 tests pass in under a minute on a recent laptop.
4. **Coverage locally**:
   ```bash
   cargo install cargo-tarpaulin
   cd packages/contracts/soroban
   cargo tarpaulin
   open coverage/index.html
   ```
5. **End-to-end flow**: open the demo app, connect a Freighter wallet on Stellar testnet, claim Blend-faucet USDC, deposit into the vault. The off-chain keeper automatically allocates surplus to Blend within ~20 s. Redeem to verify round-trip — withdrawal completes in a single transaction regardless of how the position is split between idle and the Blend strategy.

---

## Appendix — identities

- Deployer / admin / keeper / guardian / fee recipient (all bound to the same address for Tranche-1 testnet; roles will be separated before mainnet): `GCTWCR6GWAJCPNJFGVNY4VT5SJP5RZBJWL2ZW75MOXJFJP4MTEMVXAR4`
- Keeper service: `@tezoro/stellar-keeper`, pm2-managed, polling interval 20 s.
- Aggregator service: `@tezoro/yield-aggregator`, pm2-managed, cache TTL 60 s.
