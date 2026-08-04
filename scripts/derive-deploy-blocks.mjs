#!/usr/bin/env node
/**
 * derive-deploy-blocks.mjs — derive the REAL creation block of the canonical
 * ERC-8004 Identity Registry on every chain we hold an RPC URL for, and count
 * the agents registered there. This script is what produced the numbers in
 * `scripts/seed_chains.sql`; it exists so those numbers are reproducible
 * rather than folklore.
 *
 * Why this matters — the same reason the Base comment in seed_chains.sql
 * spells out. `deploy_block` is where a LOG backfill starts:
 *   - too LOW  costs time (Base from genesis, ~41M empty blocks scanned);
 *   - too HIGH silently SKIPS real registrations, and nothing complains.
 * So a deploy_block is only allowed to be a measurement, never a guess, and
 * every value is checked on BOTH SIDES of the boundary before it is emitted:
 *
 *     eth_getCode(registry, deploy_block - 1) == "0x"   (contract absent)
 *     eth_getCode(registry, deploy_block)     != "0x"   (contract present)
 *
 * A block that passes both cannot be too high, because the contract did not
 * exist one block earlier. A chain whose assertion fails emits NO row: a
 * missing chain is a visible gap, a wrong deploy_block is an invisible one.
 *
 * Method, per chain:
 *   1. eth_chainId          — the RPC serves the chain we think it does.
 *   2. eth_getCode(identity, "latest") — "0x" means not deployed here; the
 *      chain is recorded as not_deployed and emits no row.
 *   3. Exponential probe backwards from head (1, 2, 4, 8 … blocks) to bracket
 *      the deploy, then binary search inside the bracket. Nothing assumes a
 *      range: Monad's ~0.3s blocks put its head in the hundreds of millions
 *      while Ethereum's is ~24M, and the same ramp handles both.
 *   4. eth_getCode(reputation, "latest") — so the column is observed, not
 *      copied from a neighbouring row.
 *   5. Agent count by binary search on ownerOf(id) (ERC-721 selector
 *      0x6352211e); totalSupply() reverts on this contract. The id basis
 *      (0- or 1-based) is detected per chain by probing ownerOf(0).
 *
 * Limited-archive RPCs: an eth_getCode against a pruned height fails with
 * "missing trie node" / "state is not available" rather than returning "0x".
 * Reading that as "no code here" would fabricate a deploy_block far too high.
 * It is detected and the chain is reported `archive_depth_insufficient` with
 * no block, which is why that failure mode is a report and not a number.
 *
 * Plain Node >= 20, zero dependencies (global fetch).
 *
 * SECRETS: RPC URLs come from the environment only — this file contains none,
 * and none may ever be pasted into it. One env var per chain, named the way
 * the indexer names them (`crates/indexer`'s rpc_env_var() builds
 * RPC_URL_{CHAIN.to_uppercase()}), so the same variables drive both.
 *
 * Usage:
 *   # URLs live in Google Secret Manager; export them into the shell only.
 *   for c in mainnet base bsc celo robinhood worldchain arbitrum op \
 *            polygon megaeth xlayer monad gnosis; do
 *     export RPC_URL_$(echo $c | tr a-z A-Z)=$(gcloud secrets versions access \
 *       latest --secret=rpc-url-$c --project vippalo)
 *   done
 *   export RPC_URL_BILLIONS=https://rpc.billions.gateway.fm   # not on Alchemy
 *
 *   node scripts/derive-deploy-blocks.mjs                 # JSON to stdout
 *   node scripts/derive-deploy-blocks.mjs --out out.json
 *   node scripts/derive-deploy-blocks.mjs --only base,bsc  # subset
 *   node scripts/derive-deploy-blocks.mjs --no-agents      # blocks only
 *
 * A chain with no env var set is skipped and reported `no_rpc_configured`.
 */

const IDENTITY_REGISTRY = "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432";
const REPUTATION_REGISTRY = "0x8004baa17c55a88189ae136b182e5fda19de9b63";
const OWNER_OF_SELECTOR = "0x6352211e";

const RPC_TIMEOUT_MS = 20_000;
const RETRIES = 4;
const CHAIN_CONCURRENCY = 4;
const MAX_ID = 100_000_000; // ramp ceiling for the agent count; no chain is close.

/**
 * slug -> the `chain` value in the chains table, which must match the RPC env
 * var name. chainId is the EXPECTED value; it is asserted against eth_chainId
 * rather than trusted, which is what stops a mis-set URL from being measured
 * as the wrong chain.
 */
const CHAINS = [
  { slug: "mainnet",    name: "Ethereum",        chainId: 1 },
  { slug: "op",         name: "OP Mainnet",      chainId: 10 },
  { slug: "bsc",        name: "BNB Smart Chain", chainId: 56 },
  { slug: "gnosis",     name: "Gnosis",          chainId: 100 },
  { slug: "polygon",    name: "Polygon",         chainId: 137 },
  { slug: "monad",      name: "Monad",           chainId: 143 },
  { slug: "xlayer",     name: "X Layer",         chainId: 196 },
  { slug: "worldchain", name: "World Chain",     chainId: 480 },
  { slug: "megaeth",    name: "MegaETH",         chainId: 4326 },
  { slug: "robinhood",  name: "Robinhood Chain", chainId: 4663 },
  { slug: "base",       name: "Base",            chainId: 8453 },
  { slug: "arbitrum",   name: "Arbitrum One",    chainId: 42161 },
  { slug: "celo",       name: "Celo",            chainId: 42220 },
  { slug: "billions",   name: "Billions",        chainId: 45056 },
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The transport failed (network / HTTP / malformed / rate limit). */
class RpcTransportError extends Error {}
/** The node cannot serve state at that height — pruned, not absent. */
class ArchiveDepthError extends Error {}

/**
 * Errors an RPC returns when the *state* at a height is gone. These must never
 * be read as "the contract has no code there": that is the one mistake that
 * produces a deploy_block which is too high, and too high loses registrations.
 */
const ARCHIVE_PATTERNS =
  /missing trie node|state (is )?not available|older than|state at block|pruned|no historical|not found.*state|header not found|unsupported block|archive|missing state/i;

/** Overload dressed up as a JSON-RPC error; back off rather than misread it. */
const OVERLOAD_PATTERNS = /rate|limit|capacity|exceed|too many|busy|throttl|overload|503|timeout/i;

let nextId = 1;
async function rawRpc(url, method, params) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), RPC_TIMEOUT_MS);
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: nextId++, method, params }),
      signal: controller.signal,
    });
    if (!res.ok) throw new RpcTransportError(`HTTP ${res.status}`);
    let body;
    try {
      body = await res.json();
    } catch {
      throw new RpcTransportError("non-JSON response");
    }
    if (typeof body !== "object" || body === null) {
      throw new RpcTransportError("malformed JSON-RPC response");
    }
    return body;
  } catch (err) {
    if (err instanceof RpcTransportError) throw err;
    throw new RpcTransportError(err.name === "AbortError" ? "timeout" : String(err.message ?? err));
  } finally {
    clearTimeout(timer);
  }
}

/** Retrying client. Never logs the URL — it carries the API key. */
class Client {
  constructor(slug, url) {
    this.slug = slug;
    this.url = url;
    this.calls = 0;
  }
  async call(method, params) {
    let lastErr;
    for (let attempt = 0; attempt <= RETRIES; attempt++) {
      try {
        this.calls++;
        const body = await rawRpc(this.url, method, params);
        const msg = String(body.error?.message ?? "");
        if (body.error && OVERLOAD_PATTERNS.test(msg) && !ARCHIVE_PATTERNS.test(msg)) {
          throw new RpcTransportError(`rpc overloaded: ${msg}`);
        }
        return body;
      } catch (err) {
        lastErr = err;
        if (attempt < RETRIES) await sleep(500 * 2 ** attempt);
      }
    }
    throw lastErr;
  }
}

const toHexBlock = (n) => "0x" + BigInt(n).toString(16);

/**
 * true  -> the registry HAS code at that height
 * false -> the registry has NO code at that height
 * throws ArchiveDepthError if the node cannot answer for that height at all.
 */
async function hasCodeAt(client, address, block) {
  const body = await client.call("eth_getCode", [
    address,
    block === "latest" ? "latest" : toHexBlock(block),
  ]);
  if (body.error) {
    const msg = String(body.error.message ?? "");
    if (ARCHIVE_PATTERNS.test(msg)) {
      throw new ArchiveDepthError(`state unavailable at block ${block}: ${msg}`);
    }
    throw new RpcTransportError(`eth_getCode error: ${msg}`);
  }
  const code = typeof body.result === "string" ? body.result : "0x";
  return code !== "0x" && code !== "0x0" && code.length > 2;
}

async function codeSizeAt(client, address, block) {
  const body = await client.call("eth_getCode", [address, toHexBlock(block)]);
  if (body.error) throw new RpcTransportError(String(body.error.message ?? ""));
  const code = typeof body.result === "string" ? body.result : "0x";
  return Math.max(0, (code.length - 2) / 2);
}

/**
 * Earliest block at which the address has code.
 *
 * Exponential probe backwards from head to bracket the deploy — no range is
 * assumed anywhere, which is what lets one function handle Ethereum's 24M-high
 * head and Monad's ~0.3s blocks without tuning. Then binary search the
 * bracket. Returns { block, low, high } where low is known code-free.
 */
async function findDeployBlock(client, address, head) {
  // Invariant entering the ramp: code exists at `head`.
  let known = head; // has code
  let free = null; // no code
  let step = 1n;
  const headBig = BigInt(head);

  while (free === null) {
    const candidate = headBig - step;
    if (candidate <= 0n) {
      // Code all the way to genesis: a predeploy / genesis-allocated account.
      if (!(await hasCodeAt(client, address, 0))) {
        free = 0;
        break;
      }
      return { block: 0, genesis: true };
    }
    if (await hasCodeAt(client, address, candidate)) {
      known = Number(candidate);
    } else {
      free = Number(candidate);
      break;
    }
    step *= 2n;
  }

  // Binary search (free, known]: free has no code, known has code.
  let lo = free; // no code
  let hi = known; // has code
  while (hi - lo > 1) {
    const mid = lo + Math.floor((hi - lo) / 2);
    if (await hasCodeAt(client, address, mid)) hi = mid;
    else lo = mid;
  }
  return { block: hi, genesis: false };
}

const encodeOwnerOf = (id) => OWNER_OF_SELECTOR + BigInt(id).toString(16).padStart(64, "0");

async function ownedAt(client, id) {
  const body = await client.call("eth_call", [
    { to: IDENTITY_REGISTRY, data: encodeOwnerOf(id) },
    "latest",
  ]);
  if (body.error) return false; // revert -> that token id does not exist
  const result = typeof body.result === "string" ? body.result : "0x";
  return /^0x[0-9a-fA-F]{64}$/.test(result) && BigInt(result) !== 0n;
}

/** Registered agents, by the same contiguous-id binary search crates/chain uses. */
async function countAgents(client) {
  const hasZero = await ownedAt(client, 0);
  const basis = hasZero ? 0 : 1;
  if (!hasZero && !(await ownedAt(client, 1))) return { agents: 0, basis: null };

  let lo = basis; // exists
  let hi = null; // does not exist
  for (let step = 1; ; step *= 2) {
    const candidate = lo + step;
    if (candidate > MAX_ID) throw new Error("agent ramp exceeded MAX_ID");
    if (await ownedAt(client, candidate)) lo = candidate;
    else {
      hi = candidate;
      break;
    }
  }
  while (hi - lo > 1) {
    const mid = lo + Math.floor((hi - lo) / 2);
    if (await ownedAt(client, mid)) lo = mid;
    else hi = mid;
  }
  return { agents: basis === 0 ? lo + 1 : lo, basis };
}

async function probeChain(chain, opts) {
  const envVar = `RPC_URL_${chain.slug.toUpperCase()}`;
  const rec = {
    chain: chain.slug,
    name: chain.name,
    expected_chain_id: chain.chainId,
    chain_id: null,
    identity_deployed: null,
    reputation_deployed: null,
    deploy_block: null,
    code_bytes: null,
    both_sides_verified: false,
    agents: null,
    id_basis: null,
    head: null,
    status: "no_rpc_configured",
  };
  const url = process.env[envVar];
  if (!url) return rec;

  const client = new Client(chain.slug, url);
  try {
    const cid = await client.call("eth_chainId", []);
    if (cid.error) throw new RpcTransportError(`eth_chainId: ${cid.error.message}`);
    rec.chain_id = parseInt(cid.result, 16);
    if (rec.chain_id !== chain.chainId) {
      rec.status = "chain_id_mismatch";
      rec.error = `expected ${chain.chainId}, RPC reports ${rec.chain_id}`;
      return rec;
    }

    rec.identity_deployed = await hasCodeAt(client, IDENTITY_REGISTRY, "latest");
    rec.reputation_deployed = await hasCodeAt(client, REPUTATION_REGISTRY, "latest");
    if (!rec.identity_deployed) {
      rec.status = "not_deployed";
      return rec;
    }

    const bn = await client.call("eth_blockNumber", []);
    if (bn.error) throw new RpcTransportError(`eth_blockNumber: ${bn.error.message}`);
    rec.head = parseInt(bn.result, 16);

    const found = await findDeployBlock(client, IDENTITY_REGISTRY, rec.head);
    rec.deploy_block = found.block;

    // Both-sides assertion. This is the whole point: a value that passes it
    // cannot be too high, and a value that is too high loses registrations.
    if (found.genesis) {
      rec.both_sides_verified = false;
      rec.assertion = "genesis predeploy; no block below to check";
    } else {
      const belowEmpty = !(await hasCodeAt(client, IDENTITY_REGISTRY, found.block - 1));
      const atPresent = await hasCodeAt(client, IDENTITY_REGISTRY, found.block);
      rec.code_bytes = atPresent ? await codeSizeAt(client, IDENTITY_REGISTRY, found.block) : 0;
      rec.both_sides_verified = belowEmpty && atPresent;
      rec.assertion = `0 bytes at ${found.block - 1}, ${rec.code_bytes} bytes at ${found.block}`;
      if (!rec.both_sides_verified) {
        rec.status = "assertion_failed";
        return rec;
      }
    }

    if (!opts.noAgents) {
      const counted = await countAgents(client);
      rec.agents = counted.agents;
      rec.id_basis = counted.basis;
    }

    rec.status = "ok";
    return rec;
  } catch (err) {
    if (err instanceof ArchiveDepthError) {
      rec.status = "archive_depth_insufficient";
      rec.error = String(err.message);
    } else {
      rec.status = "rpc_unreachable";
      rec.error = String(err.message ?? err);
    }
    return rec;
  } finally {
    rec.rpc_calls = client.calls;
  }
}

async function mapWithConcurrency(items, limit, fn) {
  const out = new Array(items.length);
  let cursor = 0;
  const worker = async () => {
    while (cursor < items.length) {
      const i = cursor++;
      out[i] = await fn(items[i]);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return out;
}

function arg(flag) {
  const i = process.argv.indexOf(flag);
  return i === -1 ? null : process.argv[i + 1];
}

async function main() {
  const outPath = arg("--out");
  const only = arg("--only");
  const noAgents = process.argv.includes("--no-agents");
  const selected = only
    ? CHAINS.filter((c) => only.split(",").map((s) => s.trim()).includes(c.slug))
    : CHAINS;

  const results = await mapWithConcurrency(selected, CHAIN_CONCURRENCY, async (chain) => {
    const rec = await probeChain(chain, { noAgents });
    process.stderr.write(
      [
        rec.chain.padEnd(11),
        String(rec.chain_id ?? "-").padEnd(11),
        rec.status.padEnd(26),
        `deploy=${rec.deploy_block ?? "-"}`.padEnd(20),
        `agents=${rec.agents ?? "-"}`.padEnd(16),
        rec.both_sides_verified ? "verified" : "UNVERIFIED",
      ].join(" ") + "\n"
    );
    return rec;
  });

  const payload = {
    generated_at: new Date().toISOString(),
    identity_registry: IDENTITY_REGISTRY,
    reputation_registry: REPUTATION_REGISTRY,
    method: "eth_getCode binary search, both sides asserted; ownerOf binary search for agents",
    chains: results,
  };
  const json = JSON.stringify(payload, null, 2) + "\n";
  if (outPath) {
    const { writeFile } = await import("node:fs/promises");
    await writeFile(outPath, json);
    process.stderr.write(`wrote ${outPath}\n`);
  } else {
    process.stdout.write(json);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
