#!/usr/bin/env bash
# Record the week's published archives in git, through a pull request.
#
#   ./scripts/commit-index.sh                      # open the PR
#   ./scripts/commit-index.sh --merge              # open it and merge when CI is green
#
# ─────────────────────────────────────────────────────────────────────────────
# Why this is not just `git commit && git push`
# ─────────────────────────────────────────────────────────────────────────────
#
# `main` requires four status checks, and status checks only run on a pull
# request — so a direct push is rejected with `GH013: Repository rule
# violations found`, after the commit already exists locally. The recovery is
# fiddly (move the commit to a branch, reset main, push, open a PR) and it is
# the same four steps every week, at the exact moment the interesting work is
# already done and nobody wants a git puzzle.
#
# The protection is worth keeping rather than working around. A solo
# maintainer's ruleset is mostly there to stop a tired Monday from putting
# something broken on `main`, and adding a bypass for the one commit that
# happens while tired defeats it precisely.
#
# ─────────────────────────────────────────────────────────────────────────────
# What it commits, and where that comes from
# ─────────────────────────────────────────────────────────────────────────────
#
# The bucket's `runs/index.json`, which the weekly job wrote. NOT a local file:
# the job runs in a container, so the machine running this may never have seen
# the runs it is recording, and reading the local copy would commit whatever
# happened to be on this laptop.
#
# Git is where a hash becomes EVIDENCE — a value in a commit predating any
# dispute. The bucket is where it becomes USABLE. They must agree, and this is
# what makes them agree.
set -euo pipefail

BUCKET="${DATA_BUCKET:-gs://agentcount-data}"
INDEX="published-runs.json"
MERGE=false
[ "${1:-}" = "--merge" ] && MERGE=true

command -v gh >/dev/null || { echo "gh is required (brew install gh)"; exit 1; }

# Refuse to run on a dirty tree. This rewrites `published-runs.json` and
# switches branches; doing that over uncommitted work loses it.
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "!!! working tree is dirty — commit or stash first"
    git status --short
    exit 1
fi

git checkout -q main
git pull -q origin main

echo "==> fetching the published index from $BUCKET"
gcloud storage cp "$BUCKET/runs/index.json" "$INDEX"

if git diff --quiet -- "$INDEX"; then
    echo "already up to date — every published run is already in git"
    exit 0
fi

# What changed, for the branch name and the commit message. Derived from the
# diff rather than passed in, so the message cannot describe a different set of
# runs than the commit contains.
SUMMARY=$(git show "main:$INDEX" > /tmp/index-before.json 2>/dev/null && python3 - <<'PY'
import json
new = json.load(open("published-runs.json"))
try:
    old = {r["run_id"] for r in json.load(open("/tmp/index-before.json"))}
except Exception:
    old = set()
added = [r for r in new if r["run_id"] not in old]
if not added:
    print("|||")
else:
    # Newest finished date decides the census label. Two runs of one chain in
    # the same week are counted ONCE, at the later one, so the headline agrees
    # with what a reader would cite.
    by_chain = {}
    for r in sorted(added, key=lambda r: r["finished_at"] or ""):
        by_chain[r["chain"]] = r
    agents = sum(r["swept"] or 0 for r in by_chain.values())
    month = max((r["finished_at"] or "")[:7] for r in added)
    chains = ", ".join(sorted(by_chain))
    dup = len(added) - len(by_chain)
    note = f" ({dup} superseded run{'s' if dup > 1 else ''} also published)" if dup else ""
    print(f"{month}|{len(by_chain)}|{agents}|{chains}{note}")
PY
)

MONTH="${SUMMARY%%|*}"
REST="${SUMMARY#*|}"
NCHAINS="${REST%%|*}"; REST="${REST#*|}"
AGENTS="${REST%%|*}"; DETAIL="${REST#*|}"

if [ "$MONTH" = "" ]; then
    echo "index changed but no new run ids — refusing to guess at a message"
    git checkout -- "$INDEX"
    exit 1
fi

BRANCH="data/$MONTH-census"
MSG="data: publish the $MONTH census — $NCHAINS chains, $(printf "%'d" "$AGENTS" 2>/dev/null || echo "$AGENTS") agents"

echo "==> $MSG"
echo "    $DETAIL"

git checkout -q -B "$BRANCH"
git add "$INDEX"
git commit -q -m "$MSG" -m "Chains: $DETAIL

Archives are already public and immutable at
https://storage.googleapis.com/agentcount-data/runs/<run_id>.tar.zst — this commit is what makes
their hashes attestable, by putting them in a history that predates any
dispute about what an archive contained."
git push -q -u origin "$BRANCH"

gh pr create --fill --base main --head "$BRANCH" >/dev/null
URL=$(gh pr view "$BRANCH" --json url -q .url)
echo "==> $URL"

if $MERGE; then
    echo "==> waiting for checks"
    gh pr checks "$BRANCH" --watch --fail-fast >/dev/null \
        || { echo "!!! checks failed — not merging, see $URL"; exit 1; }
    gh pr merge "$BRANCH" --squash --delete-branch
    git checkout -q main && git pull -q origin main
    echo "==> merged"
else
    echo "    review it, then: gh pr merge $BRANCH --squash --delete-branch"
fi
