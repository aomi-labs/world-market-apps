#!/usr/bin/env node
/**
 * World calldata preparation sidecar. It uses the venue SDK to construct exact
 * calls, but deliberately cannot sign or broadcast them. Aomi's canonical EVM
 * stage/simulate/commit pipeline is the only execution boundary.
 */
import { createServer } from "node:http";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  AbstractSigner,
  JsonRpcProvider,
  Network,
  keccak256,
  toUtf8Bytes,
} from "ethers";
import {
  Exchange,
  OrderType,
  SwapAggregator,
} from "@wcm-inc/sdk";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
loadDotEnv(resolve(ROOT, ".env"));

const PORT = Number(process.env.WORLD_EXECUTION_PORT || 8787);
const HOST = process.env.WORLD_EXECUTION_HOST || "127.0.0.1";
const CHAIN_ID = Number(process.env.WORLD_CHAIN_ID || 2092151908);
const RPC_URL =
  process.env.WORLD_RPC_URL || "https://testnet-unifi-rpc.puffer.fi/";
const EXCHANGE =
  process.env.WORLD_EXCHANGE_ADDRESS ||
  "0xf6b54e033bb45a583aa642924bcef78b804588ae";
const DEFAULT_SLIPPAGE = Number(process.env.WORLD_EXECUTION_SLIPPAGE || 0.005);
const network = Network.from(CHAIN_ID);
const provider = new JsonRpcProvider(RPC_URL, network, { staticNetwork: true });

export class CaptureSigner extends AbstractSigner {
  constructor(transactions, connectedProvider = provider) {
    super(connectedProvider);
    this.transactions = transactions;
  }

  connect(connectedProvider) {
    return new CaptureSigner(this.transactions, connectedProvider);
  }

  async getAddress() {
    return "0x0000000000000000000000000000000000000001";
  }

  async sendTransaction(request) {
    const prepared = {
      chain_id: CHAIN_ID,
      to: String(await request.to),
      value: (request.value ?? 0n).toString(),
      data: request.data ?? "0x",
      gas_limit: request.gasLimit?.toString(),
      expires_at: Math.floor(Date.now() / 1000) + 60,
      description: "World Markets operation",
      kind: "world_markets",
      protocol: "world-markets",
    };
    this.transactions.push(prepared);
    const hash = keccak256(toUtf8Bytes(JSON.stringify(prepared)));
    return {
      hash,
      wait: async () => ({ hash, blockNumber: null, status: 1 }),
    };
  }

  async signTransaction() {
    throw new Error("calldata preparer cannot sign transactions");
  }

  async signMessage() {
    throw new Error("calldata preparer cannot sign messages");
  }

  async signTypedData() {
    throw new Error("calldata preparer cannot sign typed data");
  }
}

function preparation() {
  const transactions = [];
  const signer = new CaptureSigner(transactions);
  return {
    transactions,
    exchange: new Exchange({
      contractAddress: EXCHANGE,
      signer: { trader: signer, owner: signer },
    }),
  };
}

export function startServer() {
  const server = createServer((req, res) => {
    handle(req, res).catch((error) => {
      if (!res.headersSent) {
        send(res, 500, { ok: false, error: publicError(error) });
      }
    });
  });
  server.listen(PORT, HOST, () => {
    console.log(
      `[execution-sidecar] ${HOST}:${PORT} mode=prepare_only chain=${CHAIN_ID}`,
    );
  });
  return server;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  startServer();
}

async function handle(req, res) {
  const url = new URL(req.url || "/", `http://${HOST}`);
  if (req.method === "GET" && url.pathname === "/health") {
    send(res, 200, {
      ok: true,
      mode: "prepare_only",
      chain_id: CHAIN_ID,
      exchange: EXCHANGE,
    });
    return;
  }
  if (req.method !== "POST") {
    send(res, 404, { ok: false, error: "not found" });
    return;
  }
  const body = await readJson(req);
  try {
    if (url.pathname === "/v1/orders") {
      send(res, 200, await prepare(body, placeOrder));
      return;
    }
    if (url.pathname === "/v1/orders/cancel") {
      send(res, 200, await prepare(body, cancelOrder));
      return;
    }
    if (url.pathname === "/v1/swaps") {
      send(res, 200, await prepare(body, swap));
      return;
    }
    if (url.pathname === "/v1/loans/renew") {
      send(res, 200, await prepare(body, renewLoans));
      return;
    }
    if (url.pathname === "/v1/loans/pay-interest") {
      send(res, 200, await prepare(body, payInterest));
      return;
    }
    if (url.pathname === "/v1/loans/close") {
      send(res, 200, await prepare(body, closeLoans));
      return;
    }
    send(res, 404, { ok: false, error: "not found" });
  } catch (error) {
    send(res, 400, { ok: false, error: publicError(error) });
  }
}

async function prepare(body, build) {
  const captured = preparation();
  const detail = await build(body, captured.exchange);
  if (captured.transactions.length === 0) {
    throw new Error("venue SDK prepared no transaction");
  }
  return {
    ok: true,
    prepared: true,
    atomic_required: true,
    detail,
    transactions: captured.transactions,
    host_workflow: {
      stage: "Call evm_stage_tx once for every transaction, in order, copying every field verbatim.",
      simulate: "Call simulate_batch once with the complete ordered staged transaction list.",
      commit: "Call evm_commit_txs once with that same complete ordered list. Never split or use EOA fallback.",
    },
  };
}

async function placeOrder(body, exchange) {
  const accountId = accountIdOf(body);
  const product = String(body.product || "").toLowerCase();
  const side = normalizeSide(product, body.side);
  const quantity = required(body.quantity, "quantity");
  const orderType = resolveOrderType(body);
  const book = await orderBook(exchange, product, body);
  const price = await resolvePrice({
    product,
    side,
    price: body.price,
    orderType,
    slippage: Number(body.slippage ?? DEFAULT_SLIPPAGE),
    book,
  });
  const order = {
    accountId,
    quantity,
    price,
    type: orderType,
  };
  if (product === "lend") {
    order.interestRate = quantizeLendRate(price);
    delete order.price;
  }
  const receipt = await submitPlace(book, product, side, order);
  return receiptJson(receipt, {
    product,
    side,
    order_type: orderType === OrderType.Limit ? "limit" : "market",
    price: String(price),
  });
}

async function cancelOrder(body, exchange) {
  const accountId = accountIdOf(body);
  const product = String(body.product || "").toLowerCase();
  const side = normalizeSide(product, body.side);
  const book = await orderBook(exchange, product, body);
  let receipt;
  if (product === "lend") {
    const interestRate = quantizeLendRate(
      required(body.price ?? body.interest_rate, "interest_rate"),
    );
    const lendParams = { interestRate, accountId };
    receipt =
      side === "lend"
        ? await book.cancelLendOrder(lendParams)
        : await book.cancelBorrowOrder(lendParams);
    return receiptJson(receipt, { product, side, interest_rate: String(interestRate) });
  }
  const orderId = BigInt(required(body.order_id, "order_id"));
  const params = { orderId, accountId, silent: false };
  if (product === "spot") {
    receipt =
      side === "buy"
        ? await book.cancelBuyOrder(params)
        : await book.cancelSellOrder(params);
  } else if (product === "perp") {
    receipt =
      side === "buy"
        ? await book.cancelLongOrder(params)
        : await book.cancelShortOrder(params);
  } else {
    throw new Error(`unsupported product ${product}`);
  }
  return receiptJson(receipt, { product, side, order_id: orderId.toString() });
}

async function swap(body, exchange) {
  const router = process.env.WORLD_SWAP_ROUTER_ADDRESS;
  const helper = process.env.WORLD_PRICE_HELPER_ADDRESS;
  if (!router || !helper) {
    throw new Error(
      "WORLD_SWAP_ROUTER_ADDRESS and WORLD_PRICE_HELPER_ADDRESS must be set to swap",
    );
  }
  const tokenIn = required(body.token_in, "token_in");
  const tokenOut = required(body.token_out, "token_out");
  const amountIn = required(body.amount_in, "amount_in");
  const slippage = Number(body.slippage ?? DEFAULT_SLIPPAGE) * 100;
  const aggregator = new SwapAggregator({
    deadline: Date.now() + 5 * 60 * 1000,
    priceHelperContractAddress: helper,
    swapRouterContractAddress: router,
    exchange,
  });
  const route = await aggregator.getRouteForExactInput({
    tokenIn,
    tokenOut,
    amountIn,
  });
  const receipt = await aggregator.executeSwap({ route });
  return receiptJson(receipt, {
    route_type: route.type,
    amount_out: route.quote?.amountOut?.toString?.() ?? null,
    price_impact: route.quote?.priceImpact?.toString?.() ?? null,
    slippage: String(slippage),
  });
}

async function renewLoans(body, exchange) {
  const accountId = accountIdOf(body);
  const tokenIds = Array.isArray(body.token_ids) ? body.token_ids : [];
  if (tokenIds.length === 0) {
    throw new Error("token_ids is required");
  }
  const maxMs =
    Number(body.max_hours_remaining ?? 24) * 60 * 60 * 1000;
  const now = Date.now();
  const attempted = [];
  const skipped = [];
  for (const tokenId of tokenIds) {
    let positions;
    try {
      positions = await exchange.getAllBorrowerPositions({
        tokenId: Number(tokenId),
        accountId,
      });
    } catch (error) {
      skipped.push({
        token_id: String(tokenId),
        error: publicError(error),
      });
      continue;
    }
    for (const position of positions || []) {
      const remaining = position.endTime.getTime() - now;
      if (remaining > maxMs) {
        skipped.push({
          position_id: position.positionId.toString(),
          reason: "not_due",
        });
        continue;
      }
      const receipt = await exchange.payInterestAndFees({
        positionId: position.positionId,
        extendPeriod: true,
      });
      attempted.push({
        position_id: position.positionId.toString(),
        token_id: String(tokenId),
        prepared_call_hash: receipt.hash,
      });
    }
  }
  return {
    ok: true,
    renewed: attempted,
    skipped,
  };
}

async function payInterest(body, exchange) {
  const accountId = accountIdOf(body);
  const extendPeriod = Boolean(body.extend_period);
  const acted = [];
  const skipped = [];
  for (const position of await borrowerPositions(exchange, body, accountId)) {
    const receipt = await exchange.payInterestAndFees({
      positionId: position.positionId,
      extendPeriod,
    });
    acted.push({
      position_id: position.positionId.toString(),
      token_id: String(position.tokenId ?? body.token_id ?? ""),
      prepared_call_hash: receipt.hash,
    });
  }
  if (acted.length === 0 && skipped.length === 0) {
    throw new Error("no borrower positions to pay");
  }
  return {
    ok: true,
    prepared_call_hash: acted[0]?.prepared_call_hash ?? null,
    paid: acted,
    skipped,
  };
}

async function closeLoans(body, exchange) {
  const accountId = accountIdOf(body);
  const acted = [];
  for (const position of await borrowerPositions(exchange, body, accountId)) {
    const receipt = await exchange.closeLoan({
      positionId: position.positionId,
    });
    acted.push({
      position_id: position.positionId.toString(),
      prepared_call_hash: receipt.hash,
    });
  }
  if (acted.length === 0) {
    throw new Error("no borrower positions to close");
  }
  return {
    ok: true,
    prepared_call_hash: acted[0]?.prepared_call_hash ?? null,
    closed: acted,
  };
}

async function borrowerPositions(exchange, body, accountId) {
  const positionId = numericPositionId(body.position_id);
  if (positionId !== null) {
    return [{ positionId }];
  }
  const tokenIds = Array.isArray(body.token_ids)
    ? body.token_ids
    : body.token_id
      ? [body.token_id]
      : [];
  if (tokenIds.length === 0) {
    throw new Error("token_ids or position_id is required");
  }
  const found = [];
  for (const tokenId of tokenIds) {
    let positions;
    try {
      positions = await exchange.getAllBorrowerPositions({
        tokenId: Number(tokenId),
        accountId,
      });
    } catch (error) {
      continue;
    }
    for (const position of positions || []) {
      found.push(position);
    }
  }
  return found;
}

function quantizeLendRate(value) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) {
    throw new Error("interest_rate must be a positive number");
  }
  const ticks = Math.round(n * 10000);
  if (ticks < 1) {
    throw new Error("interest_rate is below the 0.0001 lend tick");
  }
  return (ticks / 10000).toFixed(4);
}

async function submitPlace(book, product, side, order) {
  if (product === "spot") {
    return side === "buy"
      ? book.createBuyOrder(order)
      : book.createSellOrder(order);
  }
  if (product === "perp") {
    return side === "buy"
      ? book.createLongOrder(order)
      : book.createShortOrder(order);
  }
  if (product === "lend") {
    return side === "lend"
      ? book.createLendOrder(order)
      : book.createBorrowOrder(order);
  }
  throw new Error(`unsupported product ${product}`);
}

async function orderBook(exchange, product, body) {
  const baseId = Number(required(body.base_token_id, "base_token_id"));
  if (product === "lend") {
    const book = await exchange.getLendOrderBook(baseId);
    if (!book) throw new Error(`no lend book for token ${baseId}`);
    return book;
  }
  const quoteId = Number(required(body.quote_token_id, "quote_token_id"));
  const book =
    product === "perp"
      ? await exchange.getPerpOrderBook(baseId, quoteId)
      : await exchange.getSpotOrderBook(baseId, quoteId);
  if (!book) {
    throw new Error(`no ${product} book for ${baseId}/${quoteId}`);
  }
  return book;
}

async function resolvePrice({ product, side, price, orderType, slippage, book }) {
  if (orderType === OrderType.Limit) {
    return required(price, "price");
  }
  const offer = await book.getBestOrderOffer();
  if (product === "lend") {
    const rate =
      side === "lend" ? offer.borrowInterestRate : offer.lendInterestRate;
    if (!rate || rate.isZero?.()) {
      throw new Error("empty lend book; cannot infer a market rate");
    }
    const factor = side === "lend" ? 1 - slippage : 1 + slippage;
    return rate.times(factor).toString();
  }
  const raw = side === "buy" ? offer.sellPrice : offer.buyPrice;
  if (!raw || raw.isZero?.()) {
    throw new Error("empty book; cannot infer a market price");
  }
  const factor = side === "buy" ? 1 + slippage : 1 - slippage;
  return raw.times(factor).toString();
}

function resolveOrderType(body) {
  const named = String(body.order_type || "").toLowerCase();
  if (named === "market" || named === "ioc") return OrderType.FillPartialKillRest;
  if (named === "limit") return OrderType.Limit;
  if (body.price) return OrderType.Limit;
  return OrderType.FillPartialKillRest;
}

function normalizeSide(product, raw) {
  const side = String(raw || "").toLowerCase();
  if (product === "lend") {
    if (side === "lend" || side === "sell") return "lend";
    if (side === "borrow" || side === "buy") return "borrow";
    throw new Error("lend side must be lend or borrow");
  }
  if (side === "buy" || side === "long") return "buy";
  if (side === "sell" || side === "short") return "sell";
  throw new Error("side must be buy/sell (spot) or long/short (perp)");
}

function accountIdOf(body) {
  if (body.account_id === undefined || body.account_id === null) {
    throw new Error("account_id is required");
  }
  return BigInt(body.account_id);
}

function numericPositionId(raw) {
  if (raw === undefined || raw === null || raw === "") return null;
  const value = String(raw);
  if (!/^\d+$/.test(value)) return null;
  return BigInt(value);
}

function receiptJson(receipt, extra = {}) {
  return {
    ok: true,
    prepared_call_hash: receipt?.hash ?? null,
    ...extra,
  };
}

function required(value, name) {
  if (value === undefined || value === null || value === "") {
    throw new Error(`${name} is required`);
  }
  return value;
}

function publicError(error) {
  const message = error?.shortMessage || error?.reason || error?.message || String(error);
  return message.replace(/private[_\s-]?key[^\s]*/gi, "[redacted]");
}

function send(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

function readJson(req) {
  return new Promise((resolveJson, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (chunk) => {
      size += chunk.length;
      if (size > 1_000_000) {
        reject(new Error("request too large"));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => {
      if (chunks.length === 0) {
        resolveJson({});
        return;
      }
      try {
        resolveJson(JSON.parse(Buffer.concat(chunks).toString("utf8")));
      } catch {
        reject(new Error("invalid JSON"));
      }
    });
    req.on("error", reject);
  });
}

function loadDotEnv(path) {
  if (!existsSync(path)) return;
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq < 0) continue;
    const key = trimmed.slice(0, eq).trim();
    let value = trimmed.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    if (process.env[key] === undefined) process.env[key] = value;
  }
}
