# Tranche 1 — Submission Answers

Answers to the SCF tranche-disbursement form. Network: **Stellar testnet** (`Test SDF Network ; September 2015`).

---

## 1. Tranche Deliverables

### 1. TezoroVault on Stellar testnet

An ERC-4626-style vault in Rust / Soroban at [`CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5`](https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5). Accepts USDC, mints `tUSDC-A` shares with a virtual-shares offset (no first-depositor attack), keeps a 3 % idle buffer for low-latency redemptions, and routes surplus through a pluggable strategy interface. Includes full admin / keeper / guardian role separation, a two-step admin transfer, and a 48-hour upgrade timelock (1 h minimum).

### 2. Blend v2 strategy adapter

A Soroban contract at [`CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6`](https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6) implementing Tezoro's `StrategyInterface` against Blend Protocol v2. Maps `deposit` / `withdraw` / `emergency_withdraw` to Blend's `submit()` RPC with `SupplyCollateral` / `WithdrawCollateral`, exposes `available_liquidity()` and `is_healthy()` for the vault's waterfall and healthcheck gate, and harvests BLND emissions. Any future protocol (Fluid, Yieldblox, Soroswap LP) plugs into the same interface.

### 3. End-to-end deposit flow on testnet

`deposit → keeper auto-allocate → redeem` works single-transaction, with the waterfall implementation: when the requested redeem exceeds idle, the vault pulls the shortfall from the strategy via `strategy.withdraw()` capped by `available_liquidity()`, so users exit without waiting for a keeper deallocation. Demoed through a public Freighter-based UI.

### 4. Yield aggregator serving Stellar / Blend APY data

Public endpoint `GET https://www.tezoro.io/api/agg/api/stellar/markets` returns all four Blend Testnet V2 reserves (USDC / XLM / wETH / wBTC) with supply / borrow APR, Blend-SDK-estimated APY, utilization, and pool liquidity. Zod-validated upstream, fail-fast on partial failure, 60-second cache with in-flight coalescing so an open-RPC thundering herd costs at most one upstream call.

### 5. Unit + integration test suite (> 90 % branch coverage requirement)

**194 passing Cargo tests** across the Soroban workspace, **~ 98 % line coverage** on contract code (`cargo-tarpaulin` with the Llvm engine). Covers full role matrix, ERC-4626 math surface, waterfall paths (idle-only / full-shortfall / partial-shortfall), Blend fixture integration, and mainnet-fork end-to-end scenarios. CI gates on `cargo fmt --check`, `clippy -D warnings`, and the full test run on every push.

### 6. Public source code (tagged release)

Staged for publication at `github.com/tezoroproject/tezoro-yield-aggregator-stellar`, release tag pending the final pass.

---

## 2. Additional Deliverable Verification

### Live endpoints

**Yield API:** `curl -s https://www.tezoro.io/api/agg/api/stellar/markets | jq` — current response shape:

```json
{
  "network": "testnet",
  "markets": [
    { "symbol": "USDC", "supplyApy": 0.000564, "util": 0.376 },
    { "symbol": "XLM",  "supplyApy": 1.830,    "util": 0.772 },
    { "symbol": "wETH", "supplyApy": 14.357,   "util": 0.963 },
    { "symbol": "wBTC", "supplyApy": 4.959,    "util": 0.944 }
  ]
}
```

### Stellar explorer — live contracts

- Vault: https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5
- Strategy: https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6
- Underlying Blend Testnet V2 pool: https://stellar.expert/explorer/testnet/contract/CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF

### Deployment transactions

| Step | Transaction |
|---|---|
| Vault deploy (USDC stack) | [36f31932…9ceb5](https://stellar.expert/explorer/testnet/tx/36f319323d44c11dbe9a520f182b7fa889231e565cbbee66511b838cb029ceb5) |
| Vault initialize | [53c33542…1d7e71](https://stellar.expert/explorer/testnet/tx/53c335420052c0c47a342530b5d3b5eacf2801d5522377e5627caead811d7e71) |
| Strategy WASM upload | [af0c6075…5161c](https://stellar.expert/explorer/testnet/tx/af0c6075c6a379c1bd1c7e99c605cacfa933b4d67f0e29fecaa8a91c2835161c) |
| Strategy initialize | [a4bd6f90…9d8e02](https://stellar.expert/explorer/testnet/tx/a4bd6f90883abc81c2e1bfa67655adc6d6f3e712ad54f81b6470c819359d8e02) |
| Strategy `set_min_backstop_coverage=0` (testnet-only) | [6ab4c5c6…8dcaea](https://stellar.expert/explorer/testnet/tx/6ab4c5c620df16f485b5bc9b9fa2170f47fb705d8eb409fa585bda14de8dcaea) |
| `vault.add_strategy` wiring the two together | [122511d3…89b729](https://stellar.expert/explorer/testnet/tx/122511d3158368da06989785daf6a3e6b9581d9de1d101bbe677a5405689b729) |

Full transaction history (all keeper allocations + all user deposits / redeems) is browsable on the vault and strategy explorer pages above — every `allocate`, `deallocate`, `deposit`, and `redeem` emits an event and is indexed.

### Keeper runtime activity (`pm2 logs stellar-keeper`, recent excerpts)

```
2026-04-22T14:41:18  Stellar keeper starting
                     vault:     CBKMGZJQ…RADRL5
                     strategy:  CCNVLZ23…XUCS3B6
                     intervalMs: 20000
2026-04-22T14:41:19  Stellar keeper: allocating excess idle to Blend
                     idle:    100000002      minIdle: 3000000
                     excess:  97000002
2026-04-22T14:41:26  Stellar keeper: allocation submitted
                     amount:  97000002
2026-04-22T15:02:07  Stellar keeper: allocating excess idle to Blend
                     idle:    150000000      minIdle: 4500000
                     excess:  145500000
2026-04-22T15:02:14  Stellar keeper: allocation submitted
                     amount:  145500000
```

### Source code

- Soroban contracts: `packages/contracts/soroban/contracts/{tezoro-vault, blend-strategy, mock-strategy, tezoro-common}/`
- Off-chain keeper: `packages/stellar-keeper/`
- Yield aggregator (Blend provider): `packages/yield-aggregator/src/sources/blend/`
- Demo UI: `packages/stellar-demo/`
- Completion report: [`docs/tranche-1/completion-report.md`](completion-report.md)
- Address list: [`docs/tranche-1/addresses.md`](addresses.md)

### Reproduction checklist

1. Visit either explorer link above and inspect the contract's WASM hash + deployment event.
2. `curl -s https://www.tezoro.io/api/agg/api/stellar/markets | jq` returns all four reserves, no auth.
3. Clone the repo and run `cd packages/contracts/soroban && cargo test --workspace --locked` — 194 tests pass in ~1 minute.
4. Open the demo app in a browser, connect Freighter on testnet, claim USDC from the Blend faucet, deposit into the vault. The keeper allocates within ~20 s. Redeem to confirm the waterfall round-trip in a single transaction.
