import { z } from "zod";

/**
 * Stellar keeper configuration.
 *
 * Currently tracks a single vault on Stellar testnet. Multi-vault support is
 * Tranche 2 work — kept flat here to avoid premature structure.
 */
export const STELLAR_NETWORK = {
  rpc: "https://soroban-testnet.stellar.org",
  passphrase: "Test SDF Network ; September 2015",
  explorerBaseUrl: "https://stellar.expert/explorer/testnet",
} as const;

export const STELLAR_VAULT = {
  // v7 testnet stack (2026-04-22). v7 adds the redeem waterfall so
  // user exits no longer depend on a prior keeper deallocation —
  // redeem() pulls the shortfall from strategies directly, capped by
  // strategy.available_liquidity(). StrategyUnhealthy gate and the
  // keeper's is_healthy pre-check from v6 remain unchanged.
  vaultId: "CBKMGZJQ35QOYX67JDGETUKKU5ATTLNIUJMB4NCCZNZF2LXTHERADRL5",
  strategyId: "CCNVLZ23GDSDGLMINVS4BYJXKWXMEFYFTG3UCFQN3O642E4YPXUCS3B6",
  assetId: "CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU",
  assetSymbol: "USDC",
  assetDecimals: 7,
  /**
   * Must match the value passed to vault.initialize(). The vault has no
   * public getter for this, so we mirror it here; if it ever changes on
   * a redeploy, this constant must be updated in lock-step.
   */
  idleBufferBps: 300,
} as const;

const stellarEnvSchema = z.object({
  STELLAR_KEEPER_SECRET: z
    .string()
    .startsWith("S", "Must be a Stellar Ed25519 secret seed (S...)")
    .length(56),
  STELLAR_ALLOCATE_INTERVAL_MS: z.coerce.number().int().positive().default(20_000),
  /**
   * Skip allocation runs while idle surplus is below this threshold (in
   * asset base units, e.g. 1 USDC = 10_000_000 stroops). Prevents chasing
   * dust after every tiny deposit.
   */
  STELLAR_MIN_ALLOCATE_UNITS: z.coerce.bigint().nonnegative().default(1_000_000n),
  LOG_LEVEL: z.string().default("info"),
});

export type StellarKeeperEnv = z.infer<typeof stellarEnvSchema>;

export function loadStellarEnv(): StellarKeeperEnv {
  const parsed = stellarEnvSchema.safeParse(process.env);
  if (!parsed.success) {
    const issues = parsed.error.issues.map((i) => `${i.path.join(".")}: ${i.message}`);
    throw new Error(`Invalid Stellar keeper env:\n${issues.join("\n")}`);
  }
  return parsed.data;
}
