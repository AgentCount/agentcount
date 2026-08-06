-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0019 — payments, pinned to a run like every other measurement.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- ## Why this table exists
--
-- The figures "358 agents have ever been paid, 34 via x402" were produced by a
-- one-off log study (`analysis/payments-design.md`,
-- `analysis/payments-per-chain.md`). They are not in this database, not pinned
-- to a block, and not recomputable by a sweep — which is the one property this
-- census claims for everything it publishes. A result you cannot recompute is
-- an opinion.
--
-- So the study becomes a pipeline. Rows here are run-scoped exactly like
-- `check_results`: `run_id` first, chain and agent id beside it, everything
-- read at the run's `pinned_block`. Re-running `payments <chain> <run_id>`
-- against the same chain state reproduces the same rows.
--
-- ## Why it is NOT a rung
--
-- Nothing here judges conformance to ERC-8004. "Was this agent paid" is a
-- question about token transfers, answerable against no clause of the spec, and
-- putting it in `check_results` would make it the eighth rung by placement even
-- if every word around it said otherwise. Separate table, separate binary,
-- separate section of METHODOLOGY (§8).
--
-- ## The three tables, and why three
--
--   payment_targets  the ATTRIBUTION MAP — which address is whose, on which
--                    basis, and for every address that does not qualify, why.
--   payment_scans    what was actually looked at: token, symbol, decimals,
--                    block range, direction. Absence of a row means NOT
--                    SCANNED, never "scanned and found nothing".
--   payments         one row per (target, transfer), with the verdict and the
--                    named exclusion that produced it.
--
-- The first is separate from the third because the mapping is the crux. All
-- four retractions in `analysis/payments-corrections-ledger.md` are one
-- mistake — an address was treated as an identity — and a schema that stores
-- only the transfers keeps no evidence of how they were attributed. The second
-- is separate because "we found nothing" and "we never asked" are different
-- facts, and this project spends six statuses keeping that distinction
-- everywhere else.

-- ─────────────────────────────────────────────────────────────────────────────
-- payment_targets — the attribution map, pinned.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- One row per (run, chain, agent, basis, address), INCLUDING the ones that do
-- not qualify. An agent whose `getAgentWallet` equals its owner has a row
-- saying so; deleting it would make "not attributable" indistinguishable from
-- "received nothing", which is the difference between 347 and 40,473 on Base.
CREATE TABLE payment_targets (
    run_id      UUID   NOT NULL REFERENCES runs (run_id) ON DELETE CASCADE,
    chain       TEXT   NOT NULL,
    agent_id    BIGINT NOT NULL,

    -- 'verified_wallet' = getAgentWallet(agentId), on-chain, spec-defined,
    --   changed only with an EIP-712/ERC-1271 signature proving control, and
    --   cleared automatically on NFT transfer. THE PUBLISHABLE BASIS.
    -- 'declared_wallet' = a services[] entry named `agentWallet`. Not in the
    --   spec, unverified, writable by anyone who controls the document.
    --   Recorded so the gap is measurable; never the headline.
    -- See crates/payments/src/lib.rs for the full argument and METHODOLOGY §8.
    basis       TEXT   NOT NULL CHECK (basis IN ('verified_wallet','declared_wallet')),
    address     TEXT   NOT NULL,
    -- Position in `services[]`, for a declared target. NULL for a verified one.
    -- The spec defines no precedence rule for multiple entries, so this
    -- pipeline picks none and stores the index instead — a reader who wants
    -- "first entry wins" can reconstruct it; nobody has to guess which we used.
    declared_index INT,

    -- Whether a transfer may be attributed to this agent through this address.
    eligible    BOOLEAN NOT NULL,
    -- The named reason, when it may not. Mirrors `payments::Ineligible`.
    --   wallet_unset        getAgentWallet returned zero
    --   wallet_equals_owner the contract default: no signature, not per-agent
    --   no_declared_address the document named nothing parseable
    --   burn_address        PAY-4; reachable only on the declared basis
    ineligible_reason TEXT CHECK (ineligible_reason IN
        ('wallet_unset','wallet_equals_owner','no_declared_address','burn_address')),

    -- The two facts every exclusion needs, denormalised onto the target so a
    -- payment row can be re-judged without re-reading the chain.
    owner              TEXT   NOT NULL,   -- ownerOf(agentId) at the pinned block
    registration_block BIGINT,            -- NULL = the run did not capture it

    read_at_block BIGINT NOT NULL,        -- always the run's pinned_block
    PRIMARY KEY (run_id, chain, agent_id, basis, address),

    -- Eligibility and its reason must agree. A row cannot be eligible and
    -- carry a reason, or ineligible and carry none.
    CONSTRAINT payment_targets_reason_agrees
        CHECK ((eligible AND ineligible_reason IS NULL)
            OR (NOT eligible AND ineligible_reason IS NOT NULL))
);

-- "Which agents share this address" is PAY-1's question and the first thing
-- anyone asks of this table.
CREATE INDEX idx_payment_targets_address
    ON payment_targets (run_id, basis, address) WHERE eligible;

COMMENT ON TABLE payment_targets IS
    'The attribution map for one run: which address counts as which agent''s, '
    'on which basis, and why an address does not qualify. Ineligible rows are '
    'kept deliberately — "not attributable" and "received nothing" are '
    'different facts.';

-- ─────────────────────────────────────────────────────────────────────────────
-- payment_scans — what was looked at, so absence can mean absence.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- Symbol and decimals are READ FROM THE CONTRACT and stored here, never
-- assumed. This is not defensive verbosity: BSC's USDC and USDT are 18
-- decimals, not 6, and Celo's `0x765DE816…` — documented for years as cUSD —
-- now answers `USDm` at 18. Carrying Base's 6 across all four chains would
-- have overstated BSC by a factor of 10^12
-- (`analysis/payments-per-chain.md` §2).
CREATE TABLE payment_scans (
    run_id        UUID   NOT NULL REFERENCES runs (run_id) ON DELETE CASCADE,
    chain         TEXT   NOT NULL,
    token_address TEXT   NOT NULL,
    token_symbol  TEXT   NOT NULL,       -- symbol(), verbatim
    token_decimals SMALLINT NOT NULL,    -- decimals(), verbatim
    -- The scanned range. `to_block` is always the run's pinned block; a scan
    -- that ended anywhere else is not a statement about the run's population.
    from_block    BIGINT NOT NULL,
    to_block      BIGINT NOT NULL,
    -- 'in', 'out', or 'in,out'. A row here claims only the directions it names.
    directions    TEXT   NOT NULL,
    basis         TEXT   NOT NULL CHECK (basis IN ('verified_wallet','declared_wallet')),
    targets_scanned INT  NOT NULL,
    transfers_found INT  NOT NULL,
    -- The version of the attribution rule and exclusions that judged this scan
    -- (`payments::RULE_VERSION`). A figure recomputed under a different rule is
    -- a different figure; the row has to be able to say which one it is.
    rule_version  TEXT   NOT NULL,
    scanned_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, basis, token_address)
);

COMMENT ON TABLE payment_scans IS
    'One row per (run, basis, token) actually scanned. No row means NOT '
    'SCANNED — never "scanned and found nothing". Symbol and decimals are read '
    'from the contract at the pinned block, never assumed.';

-- ─────────────────────────────────────────────────────────────────────────────
-- payments — one row per (target, transfer), verdict included.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- EXCLUDED ROWS ARE STORED. That is the point of the `included`/`exclusion`
-- pair rather than a filter at write time: a reader can reproduce the
-- uncorrected figure, apply the corrections themselves, and watch each one
-- bite. "We fixed it" is otherwise something you have to take on trust, and
-- this project's whole argument is that you should not have to.
CREATE TABLE payments (
    id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id   UUID   NOT NULL REFERENCES runs (run_id) ON DELETE CASCADE,
    chain    TEXT   NOT NULL,
    agent_id BIGINT NOT NULL,

    -- ── which address was credited, and on whose authority ────────────────
    basis            TEXT NOT NULL CHECK (basis IN ('verified_wallet','declared_wallet')),
    credited_address TEXT NOT NULL,
    -- How many agents in this run reach this address on this basis. PAY-1:
    -- one Base address is declared by 62 agents, and a payment to it cannot be
    -- assigned to any one of them. Denormalised so an agent-level count and an
    -- address-level count are both one query and neither can be mistaken for
    -- the other.
    address_reached_by INT NOT NULL DEFAULT 1,

    -- ── which token, as the contract described itself ─────────────────────
    token_address  TEXT NOT NULL,
    token_symbol   TEXT NOT NULL,
    token_decimals SMALLINT NOT NULL,

    -- ── direction ─────────────────────────────────────────────────────────
    -- 'in'  = credited_address is the Transfer's `to`
    -- 'out' = credited_address is the `from`. Stored, never counted as
    --         payment: a balance says nothing because funds received and swept
    --         out leave nothing behind, and an address whose entire history is
    --         outgoing is visibly not a payee.
    direction    TEXT NOT NULL CHECK (direction IN ('in','out')),
    counterparty TEXT NOT NULL,     -- the other end of the transfer

    -- ── the transfer itself ───────────────────────────────────────────────
    -- NUMERIC, not BIGINT: a uint256 does not fit in an i64, and narrowing it
    -- at this boundary would be the same class of mistake as assuming decimals.
    value_raw    NUMERIC NOT NULL,
    block_number BIGINT NOT NULL,
    tx_hash      TEXT NOT NULL,
    log_index    INT  NOT NULL,

    -- ── PAY-2: pre- or post-mint ──────────────────────────────────────────
    -- The agent's registration block, copied onto the row so the judgement is
    -- re-derivable without a join, and `post_mint` beside it.
    -- NULL post_mint means the registration block is unknown — which is
    -- EXCLUDED, not assumed favourable. See `exclusion = 'mint_block_unknown'`.
    agent_registration_block BIGINT,
    post_mint                BOOLEAN,

    -- ── PAY-3: was the sender a contract ──────────────────────────────────
    -- NULL means `eth_getCode` was never made. It is NOT "false". PAY-3 is the
    -- record of what assuming the favourable reading of an unmade call costs:
    -- "one operator earned 97.9%" was Morpho vault flow into 148 per-agent
    -- contracts owned by 126 addresses, none of them the registrant.
    counterparty_is_contract BOOLEAN,
    -- Whether the sender is the owner of ANY agent in this run — fleet-internal
    -- movement. Reported separately, not excluded: it is a real signal about
    -- the population and a poor one about a single agent.
    counterparty_is_run_owner BOOLEAN,

    -- ── x402: did an EIP-3009 authorization co-occur ──────────────────────
    -- `transferWithAuthorization` emits AuthorizationUsed(address indexed
    -- authorizer, bytes32 indexed nonce) from the token contract in the same
    -- transaction as the Transfer. A Transfer whose transaction also carries
    -- one from that token is an authorised (x402-style) settlement.
    eip3009_authorization BOOLEAN NOT NULL DEFAULT false,
    -- The authorizer from that event, and whether it equals this Transfer's
    -- sender. The cross-check in `analysis/x402scan-crosscheck.md` §4a tested
    -- exactly this hypothesis — that a batching contract could move funds to
    -- many recipients under one authorization and inflate a transaction-level
    -- flag — and found 8 of 8 sampled legs matching one-to-one. Storing it
    -- means the next reader does not have to sample: the join is in the row.
    eip3009_authorizer         TEXT,
    eip3009_authorizer_is_sender BOOLEAN,

    -- ── the verdict ───────────────────────────────────────────────────────
    included  BOOLEAN NOT NULL,
    -- The named rule that excluded it. Mirrors `payments::Exclusion`; the order
    -- these are applied in is fixed and tested in
    -- `crates/payments/src/exclusions.rs`.
    exclusion TEXT CHECK (exclusion IN
        ('burn_address','pre_mint','mint_block_unknown','owner_funding',
         'self_transfer','outgoing')),

    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- One transfer, one target, one row. `direction` and `credited_address` are
    -- both in the key because the same log CAN legitimately appear twice for
    -- one agent: an agent declaring two addresses that pays one from the other
    -- produces an 'out' row for the first and an 'in' row for the second, same
    -- transaction, same log index, same agent.
    CONSTRAINT payments_unique
        UNIQUE (run_id, chain, agent_id, basis, token_address, tx_hash, log_index,
                direction, credited_address),

    -- Same discipline as `payment_targets`: the verdict and its reason cannot
    -- disagree.
    CONSTRAINT payments_verdict_agrees
        CHECK ((included AND exclusion IS NULL) OR (NOT included AND exclusion IS NOT NULL)),
    -- An included row must be incoming. Nothing else is a payment TO anyone.
    CONSTRAINT payments_included_is_incoming
        CHECK (NOT included OR direction = 'in'),
    -- An included row must be post-mint and known to be. PAY-2 as a database
    -- constraint, not a convention someone can forget.
    CONSTRAINT payments_included_is_post_mint
        CHECK (NOT included OR post_mint IS TRUE)
);

-- The two counts that must never be blended: per agent, and per address.
CREATE INDEX idx_payments_agents ON payments (run_id, basis, agent_id) WHERE included;
CREATE INDEX idx_payments_addresses ON payments (run_id, basis, credited_address) WHERE included;
-- "How much did each correction remove" — the audit query.
CREATE INDEX idx_payments_exclusions ON payments (run_id, basis, exclusion);

COMMENT ON TABLE payments IS
    'One row per (attribution target, token transfer) for a run, at its pinned '
    'block. Excluded rows are KEPT with the named rule that excluded them, so '
    'the uncorrected figure stays recomputable and each correction is visible '
    'rather than asserted. Never a rung: see METHODOLOGY.md §8.';

COMMENT ON COLUMN payments.basis IS
    'Which address counted as the agent''s. verified_wallet = '
    'getAgentWallet(agentId), spec-defined and signature-verified — the only '
    'basis a published figure may be stated on. declared_wallet = the '
    'services[] convention, unverified, recorded so the gap is visible.';

COMMENT ON COLUMN payments.counterparty_is_contract IS
    'NULL means eth_getCode was never made. It does NOT mean false. See '
    'analysis/payments-corrections-ledger.md PAY-3.';
