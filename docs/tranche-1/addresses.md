# Tranche 1 — Stellar Testnet Deployments

All addresses on **Stellar testnet** (`Test SDF Network ; September 2015`).

## Tezoro Contracts

| Contract | Address |
|---|---|
| TezoroVault | [`CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5`](https://stellar.expert/explorer/testnet/contract/CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5) |
| BlendStrategy | [`CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6`](https://stellar.expert/explorer/testnet/contract/CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6) |
| Asset (Blend testnet USDC) | `CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU` |

Share token: `Tezoro USDC-A` / `tUSDC-A`. Performance fee: 15 %. Idle buffer: 3 %. Upgrade timelock: 48 h default, 1 h minimum.

### Withdrawal behavior

`redeem()` implements the standard ERC-4626 withdrawal waterfall: idle USDC is served first, and any shortfall is pulled from registered strategies via `strategy.withdraw()`, each hop capped by `strategy.available_liquidity()` so the vault never requests more than the underlying pool can actually deliver on the current ledger. User exits are single-transaction and do not depend on a prior keeper-triggered deallocation.

### Healthcheck gate

`allocate()` verifies the target strategy's `is_healthy()` before deploying funds. The strategy returns `false` if pool utilization exceeds its ceiling or backstop coverage falls below the configured floor. The off-chain keeper mirrors this pre-check before submitting allocation transactions so an unhealthy pool doesn't burn a keeper tx.

USDC is obtained by end-users from the public Blend testnet faucet (wired into the demo app). The `min_backstop_coverage` on the strategy is set to 0 % because the public Blend testnet pool is not backstopped at mainnet levels; the default 5 % threshold is retained for mainnet.

## Public Services

| Service | URL |
|---|---|
| Live API (Stellar markets) | `https://www.tezoro.io/api/agg/api/stellar/markets` |
| Demo application | Opens via the stellar-demo Vite app (see [`packages/stellar-demo/`](../../packages/stellar-demo/)) |

## External Dependencies (Blend Testnet)

Canonical addresses from [blend-capital/blend-utils/testnet.contracts.json](https://github.com/blend-capital/blend-utils/blob/main/testnet.contracts.json).

| Asset / Contract | Address |
|---|---|
| USDC | `CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU` |
| XLM | `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC` |
| BLND | `CB22KRA3YZVCNCQI64JQ5WE7UY2VAV7WFLK6A2JN3HEX56T2EDAFO7QF` |
| Blend Pool V2 (`TestnetV2`) | `CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF` |
| Blend Backstop V2 | `CBDVWXT433PRVTUNM56C3JREF3HIZHRBA64NB2C3B2UNCKIS65ZYCLZA` |
| Blend Pool Factory V2 | `CDV6RX4CGPCOKGTBFS52V3LMWQGZN3LCQTXF5RVPOOCG4XVMHXQ4NTF6` |

## Identities

Deployments are initialized by `tezoro-deployer` = [`GCTWCR6GWAJCPNJFGVNY4VT5SJP5RZBJWL2ZW75MOXJFJP4MTEMVXAR4`](https://stellar.expert/explorer/testnet/account/GCTWCR6GWAJCPNJFGVNY4VT5SJP5RZBJWL2ZW75MOXJFJP4MTEMVXAR4). For Tranche-1 testnet the same address holds admin, keeper, guardian, and fee-recipient roles; these will be separated before mainnet.
