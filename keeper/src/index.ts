import "dotenv/config";
import { startStellarAllocateLoop } from "./allocate-loop.js";
import { loadStellarEnv } from "./config.js";
import { createLogger } from "./logger.js";

async function main(): Promise<void> {
  const args = process.argv.slice(2);

  if (args.includes("--help")) {
    console.log(`
Tezoro Stellar Keeper

Deposits excess idle balance from the Tezoro vault on Stellar testnet into the
configured Blend strategy on a fixed interval. Stellar-only — no EVM code.

Usage:
  pnpm dev [--help]

Environment variables:
  STELLAR_KEEPER_SECRET         Required. Ed25519 secret seed (S...) of the
                                keeper account. Must be authorized on the vault.
  STELLAR_ALLOCATE_INTERVAL_MS  Optional. Tick interval in ms (default 20000).
  STELLAR_MIN_ALLOCATE_UNITS    Optional. Min surplus (in base units) to trigger
                                an allocate call. Default 1_000_000 (= 0.1 USDC
                                at 7 decimals).
  LOG_LEVEL                     Optional. pino log level (default: info).
`);
    process.exit(0);
  }

  const env = loadStellarEnv();
  const logger = createLogger(env.LOG_LEVEL);
  await startStellarAllocateLoop(logger);
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
