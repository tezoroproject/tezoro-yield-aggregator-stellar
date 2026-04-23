# Tranche 1 — Submission Answers

Answers to the SCF tranche-disbursement form. Network: **Stellar testnet** (`Test SDF Network ; September 2015`).

Each deliverable below mirrors the form's field structure exactly — copy-paste the corresponding block into the form.

---

## 1. Tranche Deliverables

### 1. TezoroVault Soroban contract deployed to Stellar testnet

- **Contract address:** `CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5`
- **Testnet explorer link:** https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5
- **Functionality:** ERC-4626-style Soroban contract. Accepts USDC deposits and mints `tUSDC-A` shares with a virtual-shares offset that blocks the first-depositor donation attack. Keeps a configurable idle buffer (currently 3 % of AUM) for low-latency redemptions and routes surplus through a pluggable strategy interface. Admin / keeper / guardian role separation, two-step admin transfer, and a 48-hour upgrade timelock (1-hour minimum) with an explicit cancel path.
- **Purpose:** The user-facing entry point of the product — one contract users deposit into to get Stellar lending yield. It isolates them from protocol choice, idle-buffer management, strategy swaps, and upgrade cadence, so the product surface is "one vault, one share token" even as the backend evolves.

### 2. Blend v2 strategy adapter deployed to testnet

- **Contract address:** `CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6`
- **Testnet explorer link:** https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6
- **Functionality:** Soroban contract implementing Tezoro's `StrategyInterface` against Blend Protocol v2. Maps `deposit` / `withdraw` / `emergency_withdraw` to Blend's `submit()` RPC with `SupplyCollateral` / `WithdrawCollateral`. Exposes `available_liquidity()` and `is_healthy()` — the former feeds the vault's redemption waterfall, the latter gates allocation. Harvests BLND emissions.
- **Purpose:** Turn idle vault capital into yield by plugging in Blend, the largest lending market on Stellar. It also defines the concrete `StrategyInterface` contract that every future protocol (Fluid, Yieldblox, Soroswap LP, …) must implement, so the vault itself never needs protocol-specific code paths — adding a new yield source is an adapter deploy, not a vault upgrade.

### 3. End-to-end deposit flow recorded on testnet

- **Demo video:** https://youtu.be/2S2XqDy1uhg
- **What the video demonstrates:** connect wallet → deposit USDC → `tUSDC-A` shares minted → keeper allocates principal into Blend (~20 s) → funds visible in the Blend pool → redeem → USDC returned to the user. The redemption exercises the waterfall: when requested redeem exceeds idle, the vault pulls the shortfall from the strategy via `strategy.withdraw()` capped by `available_liquidity()` in the same transaction, so the user never waits for a keeper deallocation.

### 4. Yield aggregator returning Stellar / Blend APY data

- **Public endpoint:** `GET https://www.tezoro.io/api/agg/api/stellar/markets`
- **API response screenshot source (same JSON the form expects a screenshot of):**

```json
{
  "chain": "stellar",
  "network": "testnet",
  "markets": [
    {
      "protocol": "blend",
      "poolName": "Blend Testnet V2",
      "poolId": "CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF",
      "asset": { "symbol": "USDC", "decimals": 7 },
      "supplyApr": 0.0005635, "borrowApr": 0.0016634,
      "estSupplyApy": 0.000564, "estBorrowApy": 0.001665,
      "utilization": 0.3765, "maxUtilization": 0.95
    },
    {
      "protocol": "blend", "poolName": "Blend Testnet V2",
      "asset": { "symbol": "XLM", "decimals": 7 },
      "supplyApr": 1.0509, "borrowApr": 1.5130,
      "estSupplyApy": 1.8304, "estBorrowApy": 3.5259,
      "utilization": 0.7721, "maxUtilization": 0.95
    },
    {
      "protocol": "blend", "poolName": "Blend Testnet V2",
      "asset": { "symbol": "wETH", "decimals": 7 },
      "supplyApr": 2.8046, "borrowApr": 3.3463,
      "estSupplyApy": 14.3573, "estBorrowApy": 26.9675,
      "utilization": 0.9630, "maxUtilization": 0.95
    },
    {
      "protocol": "blend", "poolName": "Blend Testnet V2",
      "asset": { "symbol": "wBTC", "decimals": 7 },
      "supplyApr": 1.8158, "borrowApr": 2.2356,
      "estSupplyApy": 4.9587, "estBorrowApy": 8.2883,
      "utilization": 0.9446, "maxUtilization": 0.95
    }
  ]
}
```

Refresh live for the screenshot: `curl -s https://www.tezoro.io/api/agg/api/stellar/markets | jq`.

- **Functionality:** Public REST endpoint returning all four Blend Testnet V2 reserves (USDC / XLM / wETH / wBTC) with supply / borrow APR, Blend-SDK-estimated APY, utilization, and pool liquidity. Zod-validated upstream, fail-fast on partial failure, 60-second cache with in-flight coalescing so a thundering herd costs at most one upstream RPC call.
- **Purpose:** A single typed, cached source for live Stellar APY / utilization / liquidity data. The vault UI, the keeper's allocation logic, and external integrators all read the same numbers here — so nobody has to run their own Soroban RPC or reimplement Blend's APY math.

### 5. Unit + integration test suite

- **Test output log (pass / fail counts):**

```
$ cargo test --workspace --locked

test result: ok.  12 passed; 0 failed; 0 ignored   # tezoro-common
test result: ok.   8 passed; 0 failed; 0 ignored   # blend-strategy (unit)
test result: ok.  15 passed; 0 failed; 0 ignored   # blend-strategy (integration)
test result: ok.  27 passed; 0 failed; 0 ignored   # mock-strategy
test result: ok.   5 passed; 0 failed; 0 ignored   # mainnet-fork (live Stellar RPC)
test result: ok.  11 passed; 0 failed; 0 ignored   # tezoro-vault (unit)
test result: ok.  23 passed; 0 failed; 0 ignored   # tezoro-vault (admin / role matrix)
test result: ok.  50 passed; 0 failed; 0 ignored   # tezoro-vault (ERC-4626 math)
test result: ok.  15 passed; 0 failed; 0 ignored   # tezoro-vault (coverage gaps / waterfall)
test result: ok.  23 passed; 0 failed; 0 ignored   # tezoro-vault (strategy integration)

TOTAL: 189 passed; 0 failed
```

Coverage (cargo-llvm-cov, nightly, `--branch` instrumentation):

```
Filename                      Regions  Functions  Lines   Branches
blend-strategy/src/events.rs   90.38%   88.89%    88.46%   -
blend-strategy/src/lib.rs      99.01%  100.00%    98.71%   91.38%
blend-strategy/src/storage.rs  99.51%   96.55%    99.23%  100.00%
mock-strategy/src/lib.rs       99.33%   96.15%    99.59%  100.00%
tezoro-common/src/lib.rs      100.00%  100.00%   100.00%   -
tezoro-vault/src/events.rs     95.50%   94.74%    94.92%   -
tezoro-vault/src/lib.rs        99.20%  100.00%    98.95%   91.82%
tezoro-vault/src/storage.rs    99.52%   98.04%    99.60%   92.86%
─────────────────────────────────────────────────────────────────
TOTAL                          99.01%   98.05%    98.60%   92.50%
```

- **Functionality:** 189 passing Cargo tests across the Soroban workspace at **92.50 % branch coverage**, 98.60 % line coverage on contract code. Covers the full role matrix (admin / keeper / guardian / fee_recipient), ERC-4626 accounting surface, redemption waterfall (idle-only / partial-shortfall / empty-strategy skip / overpull clamp / multi-strategy early-break), Blend fixture integration, and mainnet-fork end-to-end scenarios against live Stellar RPC. CI gates on `cargo fmt --check`, `clippy -D warnings`, and the full test run on every push.
- **Purpose:** Close the gap between "contracts compile" and "contracts are safe to hold user funds." Lock accounting invariants, role boundaries, and redemption guarantees against regressions before any mainnet deployment, and make every PR cheap to review — CI is the first reviewer. Branch coverage (not just line) is measured because the commitment is >90 % branch coverage, which is the stricter metric: a line can be covered while one of its conditional arms stays untested.

### 6. Source code

- **Public GitHub repository:** https://github.com/tezoroproject/tezoro-yield-aggregator-stellar
- **Functionality:** Full monorepo — Soroban contracts (vault, Blend strategy, mock strategy, common), off-chain Stellar keeper, yield aggregator backend, and demo UI. Tagged release pins the exact commit hash behind every address above.
- **Purpose:** Make the SCF deliverable independently verifiable against a specific commit hash, and open the codebase for external review, reproduction, and community contribution.

---

## 2. Additional Deliverable Verification

| Field | Value |
|---|---|
| **Product MVP** | https://stellar.tezoro.io/ |
| **Demo video** | https://youtu.be/2S2XqDy1uhg |
| **Transaction history** | See the deployment-tx table and explorer links below |
| **User logs** | See the keeper runtime log below |

### Supporting contracts

- Underlying Blend Testnet V2 pool: https://stellar.expert/explorer/testnet/contract/CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF

### Transaction history

Initial deployment + wiring:

| Step | Transaction |
|---|---|
| Vault deploy (USDC stack) | [36f31932…9ceb5](https://stellar.expert/explorer/testnet/tx/36f319323d44c11dbe9a520f182b7fa889231e565cbbee66511b838cb029ceb5) |
| Vault initialize | [53c33542…1d7e71](https://stellar.expert/explorer/testnet/tx/53c335420052c0c47a342530b5d3b5eacf2801d5522377e5627caead811d7e71) |
| Strategy WASM upload | [af0c6075…5161c](https://stellar.expert/explorer/testnet/tx/af0c6075c6a379c1bd1c7e99c605cacfa933b4d67f0e29fecaa8a91c2835161c) |
| Strategy initialize | [a4bd6f90…9d8e02](https://stellar.expert/explorer/testnet/tx/a4bd6f90883abc81c2e1bfa67655adc6d6f3e712ad54f81b6470c819359d8e02) |
| Strategy `set_min_backstop_coverage=0` (testnet-only) | [6ab4c5c6…8dcaea](https://stellar.expert/explorer/testnet/tx/6ab4c5c620df16f485b5bc9b9fa2170f47fb705d8eb409fa585bda14de8dcaea) |
| `vault.add_strategy` wiring the two contracts together | [122511d3…89b729](https://stellar.expert/explorer/testnet/tx/122511d3158368da06989785daf6a3e6b9581d9de1d101bbe677a5405689b729) |

Runtime history (every `allocate` / `deallocate` / `deposit` / `redeem` emits an event and is indexed):

- Vault operations: https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5
- Strategy operations: https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6

### User logs — keeper runtime activity

`pm2 logs stellar-keeper`, recent excerpts:

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

### Repository layout pointers

- Soroban contracts: `packages/contracts/soroban/contracts/{tezoro-vault, blend-strategy, mock-strategy, tezoro-common}/`
- Off-chain Stellar keeper: `packages/stellar-keeper/`
- Yield aggregator (Blend provider): `packages/yield-aggregator/src/sources/blend/`
- Demo UI: `packages/stellar-demo/`
- Completion report: [`docs/tranche-1/completion-report.md`](completion-report.md)
- Address list: [`docs/tranche-1/addresses.md`](addresses.md)

### Reproduction checklist

1. Visit either contract-explorer link above and inspect the WASM hash + deployment event.
2. `curl -s https://www.tezoro.io/api/agg/api/stellar/markets | jq` returns all four reserves, no auth.
3. Clone the repo and run `cd packages/contracts/soroban && cargo test --workspace --locked` — 181 tests pass in ~1 minute.
4. Open https://stellar.tezoro.io/, connect Freighter on testnet, claim USDC from the Blend faucet, deposit into the vault. The keeper allocates within ~20 s. Redeem to confirm the waterfall round-trip in a single transaction.
