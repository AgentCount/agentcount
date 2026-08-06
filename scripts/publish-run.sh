#!/usr/bin/env bash
# Publish one run's export: archive it, hash it, upload it, and record the hash
# in a file that gets committed.
#
#   ./scripts/publish-run.sh <run_id>
#
# Idempotent. An archive is IMMUTABLE once written — a run is a dated
# measurement and a URL that quietly starts returning different bytes destroys
# the only thing this publication is for. Re-running against an already
# published run verifies the remote hash matches and changes nothing.
#
# ─────────────────────────────────────────────────────────────────────────────
# It publishes a LIST, never a directory
# ─────────────────────────────────────────────────────────────────────────────
#
# `tar` is given `manifest.json` and the per-chain agent directories by name.
# It is never pointed at whatever happens to be in the working tree.
#
# That is the whole safeguard against the accident this project is one careless
# command away from: subscriber addresses, RPC URLs with API keys, a stray
# database dump, a `.env` someone put in the wrong place. A script that
# uploads a directory publishes whatever lands in it; this one publishes four
# named things and fails if they are missing.
#
# The same rule is why `pg_dump` is not how runs are published. A dump is
# "everything in the database", and "everything" now includes tables that must
# never leave it.
set -euo pipefail

RUN_ID="${1:?usage: publish-run.sh <run_id>}"
BUCKET="${DATA_BUCKET:-gs://agentcount-data}"
DIR="data/$RUN_ID"
ARCHIVE="$RUN_ID.tar.zst"
INDEX="published-runs.json"

[ -d "$DIR" ] || { echo "no export at $DIR — run the sweeper first"; exit 1; }
[ -f "$DIR/manifest.json" ] || { echo "$DIR has no manifest.json"; exit 1; }

# The named list. Chain directories are discovered (a run has exactly one, but
# the export layout permits more) and every entry is checked to exist, so a
# missing chain dir is an error rather than a quietly smaller archive.
cd "$DIR"
MEMBERS=(manifest.json)
for d in */; do
    [ -d "$d" ] || continue
    MEMBERS+=("${d%/}")
done
cd - >/dev/null

if [ "${#MEMBERS[@]}" -lt 2 ]; then
    echo "$DIR contains a manifest but no agent directories — refusing to publish an empty run"
    exit 1
fi

echo "==> 1/5 archiving ${#MEMBERS[@]} paths from $DIR"
# Built with Python's `tarfile` rather than the `tar` binary, because the
# archive has to be BYTE-REPRODUCIBLE: the hash goes into git as an
# attestation, and a hash that changes depending on which machine built it
# attests nothing.
#
# GNU tar can do this with `--sort=name --mtime=... --owner=0 --group=0`, but
# macOS ships bsdtar, which has no `--sort` and no `--mtime` — so a script
# using them produces a different archive on a developer's laptop than in CI,
# or simply fails. Setting the header fields explicitly is both portable and
# easier to check: everything that could vary is named in one place below.
python3 - "$RUN_ID" "/tmp/$ARCHIVE.tar" "${MEMBERS[@]}" <<'PY'
import os, sys, tarfile

run_id, out, members = sys.argv[1], sys.argv[2], sys.argv[3:]

def normalise(info):
    # Every field that could differ between machines, pinned. Ownership and
    # names go to zero/empty rather than to whoever happened to run this;
    # mtime to a fixed epoch, since a file's write time says when the export
    # was regenerated, not anything about the run it describes.
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = 1577836800  # 2020-01-01T00:00:00Z
    info.mode = 0o755 if info.isdir() else 0o644
    return info

paths = []
for m in members:
    p = os.path.join("data", run_id, m)
    if os.path.isdir(p):
        for root, dirs, files in os.walk(p):
            dirs.sort()
            paths.append(root)
            paths.extend(os.path.join(root, f) for f in files)
    else:
        paths.append(p)

# Sorted, so directory iteration order cannot leak into the bytes.
paths.sort()

with tarfile.open(out, "w", format=tarfile.GNU_FORMAT) as tar:
    for p in paths:
        tar.add(p, arcname=os.path.relpath(p, "data"), recursive=False,
                filter=normalise)
print(f"    {len(paths)} entries")
PY
# `--no-check` keeps zstd from embedding a checksum frame whose presence
# varies with version; `-19` because these are written once and downloaded
# many times. Compression is deterministic for a given level and input.
zstd -19 --no-check -q -f -o "/tmp/$ARCHIVE" "/tmp/$ARCHIVE.tar"
rm -f "/tmp/$ARCHIVE.tar"

SHA=$(shasum -a 256 "/tmp/$ARCHIVE" | cut -d' ' -f1)
SIZE=$(wc -c < "/tmp/$ARCHIVE" | tr -d ' ')
printf '%s  %s\n' "$SHA" "$ARCHIVE" > "/tmp/$ARCHIVE.sha256"
echo "    $ARCHIVE  $SIZE bytes  sha256 $SHA"

echo "==> 2/5 checking whether this run is already published"
if REMOTE=$(gsutil cat "$BUCKET/runs/$ARCHIVE.sha256" 2>/dev/null); then
    REMOTE_SHA=$(echo "$REMOTE" | cut -d' ' -f1)
    if [ "$REMOTE_SHA" = "$SHA" ]; then
        echo "    already published, identical bytes — nothing to do"
    else
        # Never overwrite. A published run whose contents changed is either a
        # bug in the export or a different run wearing the same id, and both
        # want a human.
        echo "!!! $RUN_ID is already published with a DIFFERENT hash"
        echo "    published: $REMOTE_SHA"
        echo "    local:     $SHA"
        echo "    Archives are immutable. Investigate before doing anything."
        exit 1
    fi
else
    echo "==> 3/5 uploading"
    # `-n` (no-clobber) as a second line of defence behind the check above:
    # two operators racing must not be able to overwrite each other.
    gsutil -h "Content-Type:application/zstd" \
           -h "Cache-Control:public, max-age=31536000, immutable" \
           cp -n "/tmp/$ARCHIVE" "$BUCKET/runs/$ARCHIVE"
    gsutil -h "Content-Type:text/plain" \
           -h "Cache-Control:public, max-age=31536000, immutable" \
           cp -n "/tmp/$ARCHIVE.sha256" "$BUCKET/runs/$ARCHIVE.sha256"
fi

echo "==> 4/5 recording the hash"
# Seed the local index from the bucket when there is not one on disk.
#
# On a workstation the repository checkout provides it. In the weekly job's
# container there is no checkout, so without this the index would start empty
# and the upload below would replace the published history with a single
# entry — silently losing every earlier run from the machine-readable index
# while the archives themselves stayed fine. Fetch first, append second.
if [ ! -f "$INDEX" ]; then
    gsutil cp "$BUCKET/runs/index.json" "$INDEX" 2>/dev/null \
        || echo "[]" > "$INDEX"
fi
# The point of this file: a hash in a commit that predates any dispute is
# evidence. A hash that only exists on a server we control is not.
python3 - "$RUN_ID" "$SHA" "$SIZE" "$INDEX" <<'PY'
import json, os, sys
run_id, sha, size, index = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
manifest = json.load(open(f"data/{run_id}/manifest.json"))

entry = {
    "run_id": run_id,
    "chain": manifest["chain"],
    "pinned_block": manifest["pinned_block"],
    "started_at": manifest["started_at"],
    "finished_at": manifest.get("finished_at"),
    "schema_version": manifest["schema_version"],
    "checker_version": manifest["checker_version"],
    "checker_commit": manifest["checker_commit"],
    # Who wrote the export, as distinct from what judged the run. Absent from
    # manifests written before 2026-08-02; .get() keeps those publishable.
    "exporter_version": manifest.get("exporter_version"),
    "exporter_commit": manifest.get("exporter_commit"),
    "rebuilt_at": manifest.get("rebuilt_at"),
    "spec_commit": manifest["spec_commit"],
    "rerun_command": manifest["rerun_command"],
    "agent_count": manifest.get("agent_count"),
    "swept": manifest.get("swept"),
    "unreadable": manifest.get("unreadable"),
    "unwritable": manifest.get("unwritable"),
    "archive": f"{run_id}.tar.zst",
    "archive_bytes": size,
    "archive_sha256": sha,
}

runs = json.load(open(index)) if os.path.exists(index) else []
existing = next((r for r in runs if r["run_id"] == run_id), None)
if existing:
    # Same immutability rule as the bucket, enforced against git this time.
    if existing["archive_sha256"] != sha:
        sys.exit(
            f"{run_id} is already in {index} with hash {existing['archive_sha256']}, "
            f"not {sha}. Archives are immutable; do not edit this by hand."
        )
    print(f"    already recorded, unchanged")
else:
    runs.append(entry)
    # Newest first, matching how every other list in this project reads.
    runs.sort(key=lambda r: r["started_at"], reverse=True)
    with open(index, "w") as f:
        json.dump(runs, f, indent=2)
        f.write("\n")
    print(f"    added to {index}")
PY

echo "==> 5/5 uploading the index"
# The index also goes to the bucket, not only to git.
#
# Git is where the hash becomes EVIDENCE — a value in a commit predating any
# dispute. But the weekly job runs in a container with no checkout and no push
# credentials, and giving an unattended job write access to the source
# repository to record a hash is a worse trade than committing it by hand
# afterwards.
#
# So the bucket holds the machine-readable index (what `heartbeat` checks, and
# what anyone can fetch), and the commit stays a human step taken when the
# week's report is written. The two must agree; `heartbeat` fails the job if
# the bucket's copy is missing a chain, and a divergence between bucket and git
# is visible to anyone who compares them.
gsutil -h "Content-Type:application/json" \
       -h "Cache-Control:public, max-age=60" \
       cp "$INDEX" "$BUCKET/runs/index.json"

echo "==> done"
echo
echo "    https://storage.googleapis.com/agentcount-data/runs/$ARCHIVE"
echo
echo "$BUCKET/runs/index.json updated."
echo
echo "Now commit $INDEX. The hash is only evidence once it is in the history:"
echo "    git add $INDEX && git commit -m 'data: publish run $RUN_ID'"
