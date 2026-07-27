# Rung 4 (`conformant`) — REQUIRED fields

Extracted verbatim from `ERC8004SPEC.md` at the commit in `SOURCE.md`
(`68fc6765761a10fb26f0692df21c8a6f9d12b1be`).
Every entry quotes the line that establishes it. If a field is not quoted
here, rung 4 does not check it.

## Scope: which document this covers

The spec defines **two** distinct off-chain JSON documents:

1. **The agent registration file** — what `agentURI` (aka ERC-721 `tokenURI`)
   resolves to (spec lines 50-124). This is the document rung 4 checks.
2. **The reputation off-chain feedback file** — an unrelated, separately
   OPTIONAL document referenced by `feedbackURI` in the Reputation Registry
   (spec lines 245-288). Its JSON comments literally say `// MUST FIELDS`
   (line 251) and `// ALL OPTIONAL FIELDS` (line 259), but this is feedback
   metadata, not agent identity metadata, and line 245 itself says "The
   **OPTIONAL** file at the URI could look like:" — the whole file is
   optional. It is **out of scope for rung 4** and is not extracted below.

## Extraction methodology (please review)

The spec's RFC 2119 terms are defined at line 36: `"MUST", "MUST NOT",
"REQUIRED", "SHALL", ... are to be interpreted as described in RFC 2119"` —
i.e. **MUST and REQUIRED are the same normative strength in this document**.
The literal string "REQUIRED" appears only once in the whole file (that
definitions line); the document does its normative work with "MUST" and,
in one place, the plain-English synonym "mandatory". The field list below
is built from those MUST/mandatory statements, not from the literal word
"REQUIRED".

The registration file's shape is introduced by a single governing sentence
(line 54): **"The registration file MUST have the following structure:"**,
followed by a JSON code block (lines 56-113). Every top-level key in that
block is treated as REQUIRED **unless** a more specific later sentence
downgrades it (OPTIONAL / SHOULD / MAY). Two such downgrades exist and are
listed under "Explicitly NOT checked" below.

---

## Rulings

### Ruling 1 — `type`, `name`, `description`, `image` remain REQUIRED (decided 2026-07-27)

**Ambiguity:** Line 54 says "The registration file MUST have the following structure" and that structure includes these four fields. Line 115 then says these fields "SHOULD ensure compatibility with ERC-721 apps". Two readings were possible:
- The SHOULD constrains what values those fields should contain (format/content compatibility), leaving presence governed by line 54's MUST.
- The SHOULD downgrades the presence requirement itself to optional.

**Decision:** The project owner has ruled that line 115's SHOULD attaches to "ensure compatibility" (a behavioral constraint), not to "are present" (a presence requirement). Line 54's MUST governs whether the fields appear in the document. All four stay REQUIRED for rung 4.

### Ruling 2 — `registrations` is NOT required, but its sub-fields are validated when present (decided 2026-07-27)

**Ambiguity:** Line 54 says "The registration file MUST have the following structure" and includes `registrations` in that structure. Line 123 then says "Agents SHOULD have at least one registration (multiple are possible), and all fields in the registration are mandatory." Two readings were possible:
- `registrations` is required to be present (per line 54), and any entry inside is mandatory (per line 123).
- Line 123's SHOULD downgrades the presence of `registrations` itself (like it does for non-empty entries), while the phrase "all fields in the registration are mandatory" still governs the contents of any entry that does exist.

**Decision:** The project owner has ruled that line 123's SHOULD applies to the `registrations` key itself (presence is recommended, not required), so a document lacking `registrations` entirely does not fail rung 4. However, the same sentence also says "all fields in the registration are mandatory"—meaning `agentId` and `agentRegistry` are required only within entries of a present `registrations` array. Rung 4 checks these two sub-fields conditionally: if `registrations` is present, every entry must have both; if absent, rung 4 does not check.

---

## Unconditionally REQUIRED (7 fields)

The registration file MUST have the following structure (line 54). These seven fields are checked on every document by rung 4:

### `type`
- JSON path: `$.type`
- Spec line: 54 (governing MUST), field appears at line 58
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 58: `"type": "https://eips.ethereum.org/EIPS/eip-8004#registration-v1",`
- Type: string (URI identifying the registration schema version)

### `name`
- JSON path: `$.name`
- Spec line: 54 (governing MUST), field appears at line 59
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 59: `"name": "myAgentName",`
- Type: string

### `description`
- JSON path: `$.description`
- Spec line: 54 (governing MUST), field appears at line 60
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 60: `"description": "A natural language description of the Agent, which MAY include what it does, how it works, pricing, and interaction methods",`
- Type: string

### `image`
- JSON path: `$.image`
- Spec line: 54 (governing MUST), field appears at line 61
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 61: `"image": "https://example.com/agentimage.png",`
- Type: string (URI)

### `services`
- JSON path: `$.services`
- Spec line: 54 (governing MUST), field appears at line 62
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 62: `"services": [`
- Type: array. Rung 4 checks only that the key is present (an array, possibly empty) — its contents are explicitly unconstrained: "The number and type of *endpoints* are fully customizable, allowing developers to add as many as they wish." (line 115)

### `x402Support`
- JSON path: `$.x402Support`
- Spec line: 54 (governing MUST), field appears at line 99
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 99: `"x402Support": false,`
- Type: boolean

### `active`
- JSON path: `$.active`
- Spec line: 54 (governing MUST), field appears at line 100
- Verbatim: "The registration file MUST have the following structure:" (line 54), schema line 100: `"active": true,`
- Type: boolean

---

## Conditionally REQUIRED (2 fields when `registrations` is present)

When a `registrations` array exists in the document, every entry in it MUST contain these two fields (line 123: "all fields in the registration are mandatory"). Rung 5 (`bound`) consumes exactly these two to validate chain-scoped registration claims.

### `registrations[].agentId`
- JSON path: `$.registrations[*].agentId`
- Spec line: 123, field appears at line 103
- Verbatim: "...and all fields in the registration are mandatory." (line 123), schema line 103: `"agentId": 22,`
- Type: integer (ERC-721 tokenId)
- **Condition:** checked only if `$.registrations` array is present; not checked if the key is absent.

### `registrations[].agentRegistry`
- JSON path: `$.registrations[*].agentRegistry`
- Spec line: 123, field appears at line 104
- Verbatim: "...and all fields in the registration are mandatory." (line 123), schema line 104: `"agentRegistry": "{namespace}:{chainId}:{identityRegistry}" // e.g. eip155:1:0x742...`
- Type: string, colon-separated `{namespace}:{chainId}:{identityRegistry}` (format defined at lines 42-45)
- **Condition:** checked only if `$.registrations` array is present; not checked if the key is absent.

---

## Explicitly NOT checked

Fields the spec marks OPTIONAL or RECOMMENDED, or whose presence itself is
SHOULD (not MUST), listed here so their absence from rung 4 is a documented
decision rather than an oversight:

- `registrations` (the key itself) — "Agents SHOULD have at least one
  registration (multiple are possible)" (line 123). The key's presence is a
  SHOULD, not a MUST; a document lacking the `registrations` key entirely
  does not fail rung 4. However, when `registrations` is present, the spec
  requires every entry to have `agentId` and `agentRegistry` (see
  "Conditionally REQUIRED" above).
- `supportedTrust` — "The *supportedTrust* field is OPTIONAL. If absent or
  empty, this ERC is used only for discovery, not for trust." (line 124)
- `services[].version` — "The *version* field in endpoints is a SHOULD,
  not a MUST." (line 115)
- `services[].skills` (OASF service entries) — inline comment, line 81:
  `"skills": [], // OPTIONAL`
- `services[].domains` (OASF service entries) — inline comment, line 82:
  `"domains": [] // OPTIONAL`

Out of scope entirely (different document, not the agent registration
file — see "Scope" above), not counted as "NOT checked" for this document:
- The Reputation Registry's off-chain feedback file fields (spec lines
  245-288), including the fields its own inline comments mark
  `// MUST FIELDS` (`agentRegistry`, `agentId`, `clientAddress`,
  `createdAt`, `value`, `valueDecimals` at feedback-file lines 252-257).
  The feedback file itself is introduced as "The **OPTIONAL** file at the
  URI could look like:" (line 245) — the whole document is optional, and
  it is not the registration document rung 4 evaluates.

---

## Step 4 sanity check

`grep -ci "required" spec/ERC8004SPEC.md` → **4** matches:

1. Line 36 — the RFC 2119 definitions sentence itself (defines the term,
   establishes MUST ≡ REQUIRED; not a field-level requirement).
2. Line 206 — "...For IPFS URIs, the hash is not required." (Reputation
   Registry `feedbackHash`; out of scope, different document/registry.)
3. Line 207 — "...so the off-chain file is not required and can be
   omitted." (the feedback file's optionality; out of scope.)
4. Line 312 — "...This field is not required for IPFS URIs."
   (`responseHash` in `appendResponse`; out of scope, Validation/Reputation
   registry function parameter, not an agent-document field.)

**Reconciliation:** none of the 4 literal "required" mentions apply to the
agent registration file rung 4 checks — three are about the Reputation
Registry's optional off-chain feedback file and its optional hash fields,
and one is the definitions sentence. This is expected, not a gap: the
document does not use the literal word "required" to mark agent-document
fields at all. It uses "MUST" — specifically the structure-establishing
sentence at line 54 ("The registration file MUST have the following
structure:"), reinforced by the resolution requirement at line 52
("*agentURI* MUST resolve to the agent registration file") — and, once,
the word "mandatory" (line 123, for the two `registrations[]` sub-fields)
for that purpose. The 9 fields checked by rung 4 (7 unconditional + 2
conditionally required) are built from those MUST/mandatory statements —
see "Extraction methodology" above — not from the 4 grep hits. The
`registrations` key itself is not checked (SHOULD, not MUST). Nothing was
missed: the low grep count against a document that clearly does define
required agent-document fields is explained entirely by this document's
word-choice (MUST over REQUIRED), not by an incomplete extraction.
