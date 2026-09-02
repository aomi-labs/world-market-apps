/**
 * Read-only UniFi exchange calls for the watch evaluator.
 * Same contract as the plugin's get_world_market / RAPV reads. No signer.
 */

import { Interface, JsonRpcProvider, Network } from "ethers";
import { recordAccount, recordMark } from "./history.js";

const DEFAULT_RPC = "https://testnet-unifi-rpc.puffer.fi/";
const DEFAULT_EXCHANGE = "0xf6b54e033bb45a583aa642924bcef78b804588ae";
const CHAIN_ID = 2092151908;
const BASE_TOKEN_ID = 1;

const iface = new Interface([
  "function getMarkPrice(uint32 tokenId) view returns (uint64 price)",
  "function riskAdjustedPortfolioValue(uint64 userId) view returns (int64)",
]);

function provider() {
  const url = process.env.WORLD_RPC_URL || DEFAULT_RPC;
  const network = Network.from(CHAIN_ID);
  return new JsonRpcProvider(url, network, { staticNetwork: true });
}

function exchange() {
  return process.env.WORLD_EXCHANGE_ADDRESS || DEFAULT_EXCHANGE;
}

/** Packed UniFi price: low 5 bits = decimal exponent. */
export function decodePrice(raw) {
  const value = BigInt(raw);
  const exponent = Number(value & 0x1fn);
  let digits = (value >> 5n).toString();
  if (exponent === 0) return digits;
  if (digits.length <= exponent) {
    digits = digits.padStart(exponent + 1, "0");
  }
  const split = digits.length - exponent;
  let out = `${digits.slice(0, split)}.${digits.slice(split)}`;
  out = out.replace(/0+$/, "");
  if (out.endsWith(".")) out += "0";
  return out;
}

export async function fetchMark(tokenId) {
  if (Number(tokenId) === BASE_TOKEN_ID) {
    return { raw: String(1 << 5), mark: "1.0" };
  }
  const data = iface.encodeFunctionData("getMarkPrice", [Number(tokenId)]);
  const result = await provider().call({ to: exchange(), data });
  const decoded = iface.decodeFunctionResult("getMarkPrice", result);
  const raw = decoded.price;
  return { raw: raw.toString(), mark: decodePrice(raw) };
}

export async function fetchRapv(accountId) {
  const data = iface.encodeFunctionData("riskAdjustedPortfolioValue", [
    BigInt(accountId),
  ]);
  const result = await provider().call({ to: exchange(), data });
  const decoded = iface.decodeFunctionResult(
    "riskAdjustedPortfolioValue",
    result,
  );
  return decoded[0].toString();
}

export async function sampleWatched({ watches, accounts }) {
  const ts = Math.floor(Date.now() / 1000);
  const seen = new Map();
  for (const watch of watches) {
    const tokenId = watch.predicate?.token_id;
    const symbol = watch.predicate?.symbol;
    if (tokenId == null || !symbol || seen.has(Number(tokenId))) continue;
    seen.set(Number(tokenId), true);
    try {
      const { mark } = await fetchMark(tokenId);
      recordMark({ symbol, token_id: Number(tokenId), mark, ts });
    } catch {
      // Fail closed: skip this tick; do not invent a mark.
    }
  }
  for (const accountId of accounts) {
    try {
      const rapv = await fetchRapv(accountId);
      recordAccount({ account_id: accountId, ts, rapv });
    } catch {
      // same: skip
    }
  }
}
