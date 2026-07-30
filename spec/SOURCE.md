# Spec provenance

- Upstream: https://github.com/erc-8004/erc-8004-contracts
- File: `ERC8004SPEC.md` (repository root)
- Commit: `68fc6765761a10fb26f0692df21c8a6f9d12b1be`
- Commit date: 2026-06-11T22:06:48+08:00
- Retrieved: 2026-07-27

`spec/ERC8004SPEC.md` is a verbatim copy at that commit. It is never edited
by hand. To update: re-fetch at a newer commit, replace both files, and bump
`SPEC_COMMIT` in `crates/checks/src/version.rs` — every check result records
which spec commit it was judged against.

## How this was retrieved

```
git clone --depth 50 https://github.com/erc-8004/erc-8004-contracts /tmp/erc8004
cd /tmp/erc8004 && git rev-parse HEAD && git log -1 --format=%cI
# -> 68fc6765761a10fb26f0692df21c8a6f9d12b1be
# -> 2026-06-11T22:06:48+08:00
cp /tmp/erc8004/ERC8004SPEC.md spec/ERC8004SPEC.md
```

Verified byte-for-byte identical via `md5` checksum of the source clone and
the copy committed to this repository (`c92192bf60e67727ce87a99305ff9a31`).

Note: `spec/ERC8004SPEC.md` is the draft text of EIP/ERC-8004 ("Trustless
Agents"), status `Draft` as of the pinned commit (see its front matter,
lines 1-12). It is not yet a Final ERC. Re-pin when the draft advances or
changes materially.

## Drift checks

A pinned spec is only honest if someone keeps checking that it still matches
the standard. Each check below records what was compared, against what, and
on what date.

### 2026-07-30 — no drift

Compared against **two independent sources**, because the upstream this copy
was taken from is not the canonical home of the standard:

| source | result |
|---|---|
| `erc-8004/erc-8004-contracts` @ HEAD | HEAD **is** `68fc676` — the repo has not moved since the pin |
| `ethereum/ERCs` @ `master`, `ERCS/erc-8004.md` | **byte-identical** |

All three files share one checksum: `c92192bf60e67727ce87a99305ff9a31`.
**Zero normative differences. Zero differences of any kind.**

The canonical text's last substantive change was **2026-01-25** ("Update
ERC-8004: Updates from community feedback", `503591a6e80e`) — five months
before the commit we pinned, and six before this check. The standard has been
stable for that whole period, and its status is still `Draft`.

Note the canonical path: ERC-8004 lives in `ethereum/ERCs` at
`ERCS/erc-8004.md`, **not** in `ethereum/EIPs` (`EIPS/eip-8004.md` returns
404). `eips.ethereum.org/EIPS/eip-8004` renders the former.

```
git clone https://github.com/erc-8004/erc-8004-contracts && git rev-parse HEAD
curl -sS https://raw.githubusercontent.com/ethereum/ERCs/master/ERCS/erc-8004.md \
  | md5   # -> c92192bf60e67727ce87a99305ff9a31
```
