import assert from "node:assert/strict";
import test from "node:test";

import { CaptureSigner } from "../src/server.js";

test("captures calldata without signing or broadcasting", async () => {
  const transactions = [];
  const signer = new CaptureSigner(transactions, null);
  const response = await signer.sendTransaction({
    to: "0x2222222222222222222222222222222222222222",
    value: 7n,
    data: "0x1234",
    gasLimit: 21_000n,
  });

  assert.equal(transactions.length, 1);
  assert.deepEqual(
    {
      to: transactions[0].to,
      value: transactions[0].value,
      data: transactions[0].data,
      gas_limit: transactions[0].gas_limit,
    },
    {
      to: "0x2222222222222222222222222222222222222222",
      value: "7",
      data: "0x1234",
      gas_limit: "21000",
    },
  );
  assert.equal((await response.wait()).status, 1);
  await assert.rejects(() => signer.signTransaction({}), /cannot sign/);
  await assert.rejects(() => signer.signMessage("test"), /cannot sign/);
});
