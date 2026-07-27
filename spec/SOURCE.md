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
