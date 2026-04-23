# @tezoro/stellar-keeper

Stellar/Soroban allocation keeper for the Tezoro vault on Stellar testnet.
Polls the vault's idle balance and, whenever it exceeds the configured
idle-buffer, calls `vault.allocate(keeper, strategy, excess)` to deploy
surplus USDC into the Blend pool.

This package is intentionally Stellar-only: no EVM code, no `viem`, no
shared runtime with `@tezoro/keeper`. It is self-contained so it can run
as an isolated pm2 process (and be vendored / open-sourced on its own).

## Run

```bash
pnpm --filter @tezoro/stellar-keeper dev
```

Required env: `STELLAR_KEEPER_SECRET` (Ed25519 seed of an account authorized
on the vault). See `src/index.ts --help` for the full list.

## Configuration

The vault, strategy, and SAC asset contract IDs are pinned in
[`src/config.ts`](src/config.ts). They must be updated in lock-step with any
testnet redeploy.
