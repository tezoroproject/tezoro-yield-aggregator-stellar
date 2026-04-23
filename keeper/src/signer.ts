import {
  Address,
  Contract,
  type Keypair,
  nativeToScVal,
  rpc,
  scValToNative,
  TimeoutInfinite,
  TransactionBuilder,
  type xdr,
} from "@stellar/stellar-sdk";
import { STELLAR_NETWORK } from "./config.js";

const sorobanRpc = new rpc.Server(STELLAR_NETWORK.rpc);

/**
 * Max fee (in stroops) we're willing to pay for a contract invocation. Chosen
 * generously so that the vault + strategy + Blend inner calls all fit even
 * when resource fees spike — Soroban only consumes what's actually needed.
 */
const INVOKE_FEE_STROOPS = "10000000";

/** Nominal fee for a read-only simulateTransaction — the call never lands. */
const SIMULATION_FEE_STROOPS = "100";

/** Transaction confirmation polling — caps out at 60s, enough for Soroban finality. */
const TX_POLL_MAX_ATTEMPTS = 30;
const TX_POLL_INTERVAL_MS = 2_000;

export function getRpc(): rpc.Server {
  return sorobanRpc;
}

export function addressArg(addr: string): xdr.ScVal {
  return nativeToScVal(Address.fromString(addr), { type: "address" });
}

export function i128Arg(value: bigint): xdr.ScVal {
  return nativeToScVal(value, { type: "i128" });
}

/**
 * Simulate a read-only contract call. The source account only needs to
 * exist on-chain — we use the keeper's own address since it's guaranteed
 * funded.
 */
export async function simulateRead(
  contractId: string,
  method: string,
  args: xdr.ScVal[],
  sourceAddress: string,
): Promise<unknown> {
  const account = await sorobanRpc.getAccount(sourceAddress);
  const contract = new Contract(contractId);
  const tx = new TransactionBuilder(account, {
    fee: SIMULATION_FEE_STROOPS,
    networkPassphrase: STELLAR_NETWORK.passphrase,
  })
    .addOperation(contract.call(method, ...args))
    .setTimeout(30)
    .build();
  const sim = await sorobanRpc.simulateTransaction(tx);
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(`simulateTransaction: ${sim.error}`);
  }
  if ("result" in sim && sim.result) {
    return scValToNative(sim.result.retval);
  }
  throw new Error("simulateTransaction: no return value");
}

/**
 * Build, prepare, sign (with the keeper's keypair), submit, and wait for
 * finalization of a Soroban contract invocation.
 */
export async function invokeWithKeypair(params: {
  contractId: string;
  method: string;
  args: xdr.ScVal[];
  keypair: Keypair;
}): Promise<unknown> {
  const { contractId, method, args, keypair } = params;

  const account = await sorobanRpc.getAccount(keypair.publicKey());
  const contract = new Contract(contractId);
  const built = new TransactionBuilder(account, {
    fee: INVOKE_FEE_STROOPS,
    networkPassphrase: STELLAR_NETWORK.passphrase,
  })
    .addOperation(contract.call(method, ...args))
    .setTimeout(TimeoutInfinite)
    .build();

  const prepared = await sorobanRpc.prepareTransaction(built);
  prepared.sign(keypair);

  const send = await sorobanRpc.sendTransaction(prepared);
  if (send.status === "ERROR") {
    throw new Error(`sendTransaction: ${JSON.stringify(send.errorResult)}`);
  }

  const hash = send.hash;
  for (let attempt = 0; attempt < TX_POLL_MAX_ATTEMPTS; attempt += 1) {
    const status = await sorobanRpc.getTransaction(hash);
    if (status.status === "SUCCESS") {
      const rv = status.returnValue;
      return rv ? scValToNative(rv) : undefined;
    }
    if (status.status === "FAILED") {
      throw new Error(`Soroban tx ${hash} failed`);
    }
    await new Promise((resolve) => setTimeout(resolve, TX_POLL_INTERVAL_MS));
  }
  throw new Error(
    `Soroban tx ${hash} timed out after ${(TX_POLL_MAX_ATTEMPTS * TX_POLL_INTERVAL_MS) / 1000}s`,
  );
}
