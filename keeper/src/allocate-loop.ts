import { Keypair } from "@stellar/stellar-sdk";
import { loadStellarEnv, STELLAR_NETWORK, STELLAR_VAULT } from "./config.js";
import type { Logger } from "./logger.js";
import { addressArg, i128Arg, invokeWithKeypair, simulateRead } from "./signer.js";

/**
 * Query the vault's own SAC balance — that's the idle portion of total
 * assets. Anything above the vault's internal idle_buffer is a candidate
 * for allocation.
 */
async function readIdleBalance(keeperAddress: string): Promise<bigint> {
  const value = await simulateRead(
    STELLAR_VAULT.assetId,
    "balance",
    [addressArg(STELLAR_VAULT.vaultId)],
    keeperAddress,
  );
  if (typeof value !== "bigint") {
    throw new Error(`asset.balance(vault) returned non-bigint: ${String(value)}`);
  }
  return value;
}

async function readTotalAssets(keeperAddress: string): Promise<bigint> {
  const value = await simulateRead(STELLAR_VAULT.vaultId, "total_assets", [], keeperAddress);
  if (typeof value !== "bigint") {
    throw new Error(`vault.total_assets returned non-bigint: ${String(value)}`);
  }
  return value;
}

/**
 * Pre-check the strategy's own health gate before attempting allocate. The
 * vault's `allocate()` will already reject with `StrategyUnhealthy` when
 * the adapter reports itself unhealthy, but pre-checking here turns what
 * would be a submitted-and-reverted transaction into a clean skip — the
 * tick doesn't count against the consecutive-failure fuse that drives
 * pm2-restart semantics.
 */
async function readStrategyHealthy(keeperAddress: string): Promise<boolean> {
  const value = await simulateRead(STELLAR_VAULT.strategyId, "is_healthy", [], keeperAddress);
  if (typeof value !== "boolean") {
    throw new Error(`strategy.is_healthy returned non-boolean: ${String(value)}`);
  }
  return value;
}

const BPS_DENOMINATOR = 10_000n;

/**
 * Exit the process after this many consecutive tick failures. pm2 will
 * restart us; the logs show the real signal instead of silently looping
 * on a broken RPC / revoked keeper auth / stale contract address.
 */
const MAX_CONSECUTIVE_FAILURES = 5;

type TickResult =
  | { action: "idle-below-buffer"; idle: bigint; minIdle: bigint }
  | { action: "below-threshold"; excess: bigint; threshold: bigint }
  | { action: "strategy-unhealthy"; excess: bigint }
  | { action: "allocated"; amount: bigint; txResult: unknown };

async function tick(
  keypair: Keypair,
  minAllocateUnits: bigint,
  logger: Logger,
): Promise<TickResult> {
  const keeperAddress = keypair.publicKey();
  const [idle, totalAssets] = await Promise.all([
    readIdleBalance(keeperAddress),
    readTotalAssets(keeperAddress),
  ]);

  const minIdle = (totalAssets * BigInt(STELLAR_VAULT.idleBufferBps)) / BPS_DENOMINATOR;
  if (idle <= minIdle) {
    return { action: "idle-below-buffer", idle, minIdle };
  }

  const excess = idle - minIdle;
  if (excess < minAllocateUnits) {
    return { action: "below-threshold", excess, threshold: minAllocateUnits };
  }

  // Only touch Blend when the adapter's self-reported health gate is green.
  // The vault's own `allocate()` enforces the same invariant; this pre-check
  // just keeps us from submitting a doomed tx on every tick.
  const healthy = await readStrategyHealthy(keeperAddress);
  if (!healthy) {
    return { action: "strategy-unhealthy", excess };
  }

  logger.info(
    { idle: idle.toString(), minIdle: minIdle.toString(), excess: excess.toString() },
    "Stellar keeper: allocating excess idle to Blend",
  );

  const txResult = await invokeWithKeypair({
    contractId: STELLAR_VAULT.vaultId,
    method: "allocate",
    args: [addressArg(keeperAddress), addressArg(STELLAR_VAULT.strategyId), i128Arg(excess)],
    keypair,
  });

  return { action: "allocated", amount: excess, txResult };
}

export async function startStellarAllocateLoop(logger: Logger): Promise<void> {
  const env = loadStellarEnv();
  const keypair = Keypair.fromSecret(env.STELLAR_KEEPER_SECRET);
  const keeperAddress = keypair.publicKey();

  logger.info(
    {
      vault: STELLAR_VAULT.vaultId,
      strategy: STELLAR_VAULT.strategyId,
      asset: STELLAR_VAULT.assetSymbol,
      keeper: keeperAddress,
      intervalMs: env.STELLAR_ALLOCATE_INTERVAL_MS,
      minAllocateUnits: env.STELLAR_MIN_ALLOCATE_UNITS.toString(),
      rpc: STELLAR_NETWORK.rpc,
    },
    "Stellar keeper starting",
  );

  let running = false;
  let consecutiveFailures = 0;
  let shuttingDown = false;

  const runTick = async () => {
    if (running || shuttingDown) return;
    running = true;
    try {
      const result = await tick(keypair, env.STELLAR_MIN_ALLOCATE_UNITS, logger);
      consecutiveFailures = 0;
      if (result.action === "allocated") {
        logger.info({ amount: result.amount.toString() }, "Stellar keeper: allocation submitted");
      } else if (result.action === "idle-below-buffer") {
        logger.debug(
          { idle: result.idle.toString(), minIdle: result.minIdle.toString() },
          "Stellar keeper: idle at or below buffer, skipping",
        );
      } else if (result.action === "below-threshold") {
        logger.debug(
          { excess: result.excess.toString(), threshold: result.threshold.toString() },
          "Stellar keeper: excess below threshold, skipping",
        );
      } else {
        // strategy-unhealthy — warn loudly (on-chain protocol state deserves
        // attention) but don't count against the failure fuse: this is a
        // known, legitimate skip signal, not a broken system.
        logger.warn(
          { excess: result.excess.toString(), strategy: STELLAR_VAULT.strategyId },
          "Stellar keeper: strategy.is_healthy()=false, skipping allocation",
        );
      }
    } catch (err) {
      consecutiveFailures += 1;
      logger.error(
        { err, consecutiveFailures, limit: MAX_CONSECUTIVE_FAILURES },
        "Stellar keeper: tick failed",
      );
      if (consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) {
        logger.fatal(
          { consecutiveFailures },
          "Stellar keeper: exceeded consecutive failure limit, exiting for pm2 restart",
        );
        shuttingDown = true;
        process.exit(1);
      }
    } finally {
      running = false;
    }
  };

  await runTick();
  const timer = setInterval(runTick, env.STELLAR_ALLOCATE_INTERVAL_MS);

  const shutdown = (signal: NodeJS.Signals) => {
    if (shuttingDown) return;
    shuttingDown = true;
    logger.info({ signal }, "Stellar keeper: received signal, shutting down");
    clearInterval(timer);
    // Let the currently-running tick (if any) finish; then exit cleanly.
    const shutdownCheckMs = 200;
    const waitForTick = setInterval(() => {
      if (!running) {
        clearInterval(waitForTick);
        process.exit(0);
      }
    }, shutdownCheckMs);
  };

  process.on("SIGTERM", shutdown);
  process.on("SIGINT", shutdown);

  // Run forever (until a signal fires).
  await new Promise(() => {});
}
