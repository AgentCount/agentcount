#!/usr/bin/env bash
# Put the weekly census on a schedule: a Cloud Run Job plus Cloud Scheduler.
#
# ─────────────────────────────────────────────────────────────────────────────
# READ THIS BEFORE RUNNING IT.
# ─────────────────────────────────────────────────────────────────────────────
#
# This script is deliberately NOT run as part of any deploy, and it has not
# been run. Three of its consequences are decisions rather than mechanics, and
# they belong to whoever pays the bills:
#
# 1. **RPC cost.** A four-chain sweep reads every agent. BNB Chain alone is
#    244,208 of them, and this would run 52 times a year. On a metered RPC
#    plan that is the single largest recurring cost this project has, and it
#    is not visible anywhere until the invoice. Measure one manual sweep's
#    request count against your provider's dashboard before scheduling 52.
#
# 2. **Rung 6 sends real traffic to other people's servers**, on a schedule,
#    forever. `METHODOLOGY.md` §6 commits to one request per distinct URL and
#    a 500-URL-per-host budget, which is ~14,494 requests per run. That is
#    defensible weekly; it would not be defensible hourly, and nothing in the
#    code stops someone setting a tighter cron.
#
# 3. **Secrets.** The RPC URLs contain API keys. They go in Secret Manager,
#    never in the job's plain environment, or they appear in `gcloud run jobs
#    describe` output and in anyone's terminal scrollback.
#
# ─────────────────────────────────────────────────────────────────────────────
# What it creates
# ─────────────────────────────────────────────────────────────────────────────
#
#   Artifact Registry image   built from Dockerfile.sweep (one image, all jobs)
#   Cloud Run Job             agentcount-sweep       — nine chains, Monday 06:00 UTC
#   Cloud Run Job             agentcount-sweep-bsc   — BNB Chain,   Tuesday 06:00 UTC
#   Cloud Run Job             agentcount-sweep-base  — Base,        Wednesday 06:00 UTC
#   Cloud Run Job             agentcount-sellers     — the Seller Census (§10),
#                                                      Thursday 06:00 UTC
#   Cloud Scheduler jobs      one per Cloud Run job
#
# Four days rather than one, because a chain that runs long must only ever be
# able to fail itself. See the BNB Chain note below, and Base's beside it.
#
# ─────────────────────────────────────────────────────────────────────────────
# Why BNB Chain has a job of its own
# ─────────────────────────────────────────────────────────────────────────────
#
# 24 hours is the Cloud Run Jobs MAXIMUM task timeout, not a setting — there is
# nothing left to raise. On 2026-08-10 eleven chains in one job hit it: the ten
# smaller chains took 10h30m, BSC started with 13h left, needed 12h44m, and was
# killed at 96.4% with ~6,600 agents unswept.
#
# It was not close to a fluke. RPC throughput varies about 3x run to run — BSC
# took 3h47m on 2026-08-05 and 11h19m on 2026-07-29 — so the same job finishes
# comfortably one week and times out the next, which is the worst kind of
# schedule to own.
#
# Split, each job gets the full 24h window for its own work: ten chains in
# ~10h30m, BSC in ~13h. A day apart rather than hours, so a slow Monday cannot
# push into Tuesday's run and have the two contend for the same RPC endpoints.
#
# **No retries, on purpose.** A sweep that failed halfway has written partial
# rows and holds a `running` run. The resume path (`SWEEP_RESUME=<run_id>`)
# exists and is correct, but it must be a decision — an automatic retry would
# re-pin a new block and produce a second run for the same week, which makes
# the delta compare the wrong pair.
#
# 24 hours is the task timeout because Cloud Run Jobs cap there. A four-chain
# sweep has not been timed end to end at this scale; if it approaches the cap,
# split the job per chain rather than raising anything.
set -euo pipefail

PROJECT="${PROJECT:?set PROJECT}"
REGION="${REGION:-europe-north1}"
INSTANCE="${INSTANCE:?set INSTANCE, e.g. project:region:agentcount-db}"
REPO="${REPO:-agentcount}"
IMAGE="${REGION}-docker.pkg.dev/${PROJECT}/${REPO}/sweep:latest"

# The commit being deployed, stamped into every run this image produces.
# A dirty tree is marked as such rather than claiming to be the commit: a sweep
# from uncommitted code is not reproducible and the stamp should say so.
COMMIT="$(git rev-parse HEAD)"
git diff --quiet && git diff --cached --quiet || COMMIT="$COMMIT-dirty"
echo "==> 0/4 stamping checker_commit=$COMMIT"
case "$COMMIT" in
    *-dirty)
        echo "!!! deploying from a DIRTY tree — every run will be stamped -dirty."
        echo "    Commit first unless this is deliberate."
        read -r -p "    Continue? [y/N] " reply
        [ "$reply" = "y" ] || exit 1
        ;;
esac

echo "==> 1/4 build and push the sweep image"
# Built from Dockerfile.sweep, which is separate from the API's image on
# purpose: the public-facing container must not carry a binary that holds RPC
# credentials and writes to every table.
# The config goes to a real file, not stdin. `--config=-` is rejected by
# current gcloud, which takes the dash literally and fails with
# "Unable to read file [-]" — an error that reads like a missing file rather
# than an unsupported form, and cost an afternoon the first time.
# A directory, so the file can keep its .yaml suffix: gcloud infers the config
# format from the extension and rejects a bare temp name.
BUILD_TMP="$(mktemp -d)"
BUILD_CONFIG="$BUILD_TMP/cloudbuild.yaml"
trap 'rm -rf "$BUILD_TMP"' EXIT
# `--build-arg CHECKER_COMMIT` is the whole reason `$COMMIT` is computed above.
# It was computed, printed and prompted on — and then never passed, so
# `CHECKER_COMMIT_OVERRIDE` was empty in the image, `crates/sweeper/build.rs`
# fell through to `git rev-parse` inside a tarball with no `.git`, and every
# scheduled run of the 2026-08 census stamped `checker_commit: unknown`.
#
# The second step is why that went unnoticed: nothing ever asked the image what
# it was. It asks now, and a build that cannot answer never becomes an image
# anybody can schedule.
cat > "$BUILD_CONFIG" <<EOF
steps:
  - name: gcr.io/cloud-builders/docker
    args: ['build', '--build-arg', 'CHECKER_COMMIT=${COMMIT}',
           '-f', 'Dockerfile.sweep', '-t', '${IMAGE}', '.']
  - name: '${IMAGE}'
    entrypoint: /bin/sh
    args:
      - -c
      - |
        stamp="\$(sweeper --version)"
        echo "image reports: \$stamp"
        case "\$stamp" in
          *"checker_commit=${COMMIT}") echo "stamp verified" ;;
          *) echo "FATAL: expected checker_commit=${COMMIT}"; exit 1 ;;
        esac
images: ['${IMAGE}']
EOF

gcloud builds submit --project "$PROJECT" --region "$REGION" \
    --config="$BUILD_CONFIG" .

echo "==> 2/4 secrets"
# Create these once, by hand, and never through this script — a secret written
# by a script is a secret that was on a command line:
#
#   printf %s "$RPC" | gcloud secrets create rpc-url-base --data-file=- --project "$PROJECT"
#
# The script only checks they exist, so a missing one fails here rather than
# at 06:00 on a Monday.
for s in rpc-url-base rpc-url-bsc rpc-url-mainnet rpc-url-celo agentcount-db-url; do
    gcloud secrets describe "$s" --project "$PROJECT" >/dev/null \
        || { echo "missing secret: $s — create it by hand, see above"; exit 1; }
done

echo "==> 3/4 the Cloud Run Jobs"
gcloud run jobs deploy agentcount-sweep \
    --project "$PROJECT" --region "$REGION" \
    --image "$IMAGE" \
    --set-cloudsql-instances "$INSTANCE" \
    --set-secrets "DATABASE_URL=agentcount-db-url:latest,\
RPC_URL_BASE=rpc-url-base:latest,\
RPC_URL_BSC=rpc-url-bsc:latest,\
RPC_URL_MAINNET=rpc-url-mainnet:latest,\
RPC_URL_CELO=rpc-url-celo:latest" \
    --set-env-vars "DATA_BUCKET=gs://agentcount-data,SWEEP_CHAINS=${SWEEP_CHAINS:-op polygon arbitrum gnosis celo xlayer megaeth billions mainnet}${HEARTBEAT_URL:+,HEARTBEAT_URL=$HEARTBEAT_URL}" \
    --task-timeout 24h \
    --max-retries 0 \
    --memory 2Gi --cpu 2 \
    --parallelism 1 --tasks 1

# BNB Chain, same image and the same database, its own timeout window. Only the
# one RPC secret it needs: a job that cannot reach the other chains cannot
# accidentally sweep them if SWEEP_CHAINS is ever mistyped.
gcloud run jobs deploy agentcount-sweep-bsc \
    --project "$PROJECT" --region "$REGION" \
    --image "$IMAGE" \
    --set-cloudsql-instances "$INSTANCE" \
    --set-secrets "DATABASE_URL=agentcount-db-url:latest,RPC_URL_BSC=rpc-url-bsc:latest" \
    --set-env-vars "DATA_BUCKET=gs://agentcount-data,SWEEP_CHAINS=bsc${HEARTBEAT_URL:+,HEARTBEAT_URL=$HEARTBEAT_URL}" \
    --task-timeout 24h \
    --max-retries 0 \
    --memory 8Gi --cpu 4 \
    --parallelism 1 --tasks 1

# Base, its own job and its own day, for the reason BNB Chain got one.
#
# Base blew the 24-hour task timeout on 2026-08-24 and again the week before,
# both times from RPC timeouts on its own endpoint rather than from its size —
# 67,951 agents against BSC's 300,552. Both times it had to be finished by
# hand with SWEEP_RESUME, and both times it took the other nine chains' job
# down with it: a run that dies at the cap leaves a `running` row, no
# heartbeat, and a week's census that says `failed` while nine chains are
# actually complete.
#
# Alone, it gets the full 24-hour window for one chain and cannot fail
# anything but itself. Wednesday rather than Monday so a slow Base cannot
# collide with BNB Chain's Tuesday.
gcloud run jobs deploy agentcount-sweep-base \
    --project "$PROJECT" --region "$REGION" \
    --image "$IMAGE" \
    --set-cloudsql-instances "$INSTANCE" \
    --set-secrets "DATABASE_URL=agentcount-db-url:latest,RPC_URL_BASE=rpc-url-base:latest" \
    --set-env-vars "DATA_BUCKET=gs://agentcount-data,SWEEP_CHAINS=base${HEARTBEAT_URL:+,HEARTBEAT_URL=$HEARTBEAT_URL}" \
    --task-timeout 24h \
    --max-retries 0 \
    --memory 2Gi --cpu 2 \
    --parallelism 1 --tasks 1

# The Seller Census (METHODOLOGY §10), same image, different entrypoint.
#
# It reads other people's catalogs and probes thousands of other people's
# endpoints, so it is bounded by politeness rather than by chain throughput:
# the first full sweep took about two hours end to end. Thursday keeps its
# Base RPC scan off the day Base's own chain sweep runs, so the two cannot
# contend for the same endpoint's quota.
#
# `--command seller-sweep` rather than a second image: one release, one copy
# of the rules crate. See Dockerfile.sweep.
gcloud run jobs deploy agentcount-sellers \
    --project "$PROJECT" --region "$REGION" \
    --image "$IMAGE" \
    --command seller-sweep \
    --set-cloudsql-instances "$INSTANCE" \
    --set-secrets "DATABASE_URL=agentcount-db-url:latest,RPC_URL_BASE=rpc-url-base:latest" \
    --task-timeout 6h \
    --max-retries 0 \
    --memory 2Gi --cpu 2 \
    --parallelism 1 --tasks 1

echo "==> 4/4 the schedules"
# Monday 06:00 UTC. Weekly rather than daily because the delta's unit is a
# week, and because rung 6's traffic to third-party servers is only defensible
# at this cadence.
# Cloud Scheduler does NOT support europe-north1, where the jobs run. The
# scheduler location and the job's region are independent — the trigger is an
# HTTPS call to the regional Run endpoint — so europe-west1 here is not a
# mistake and must not be "corrected" to match REGION.
SCHED_LOCATION="${SCHED_LOCATION:-europe-west1}"

# A dedicated service account holding `run.invoker` on these two jobs and
# nothing else.
#
# NOT `gcloud config get-value account`, which this script used to pass: that is
# whoever happened to run the deploy. A schedule authenticated as a person
# breaks when they lose access, rotate credentials, or leave, and it grants the
# trigger everything that person can do rather than the one permission it needs.
SCHED_SA="agentcount-scheduler@${PROJECT}.iam.gserviceaccount.com"
gcloud iam service-accounts describe "$SCHED_SA" --project "$PROJECT" >/dev/null 2>&1 \
    || gcloud iam service-accounts create agentcount-scheduler \
         --display-name="Triggers the weekly AgentCount census" --project "$PROJECT"
for job in agentcount-sweep agentcount-sweep-bsc agentcount-sweep-base agentcount-sellers; do
    gcloud run jobs add-iam-policy-binding "$job" \
        --region "$REGION" --project "$PROJECT" \
        --member "serviceAccount:$SCHED_SA" --role roles/run.invoker --quiet >/dev/null
done

gcloud scheduler jobs create http agentcount-weekly-sweep \
    --project "$PROJECT" --location "$SCHED_LOCATION" \
    --schedule "0 6 * * 1" --time-zone "Etc/UTC" \
    --uri "https://${REGION}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${PROJECT}/jobs/agentcount-sweep:run" \
    --http-method POST \
    --oauth-service-account-email "$SCHED_SA" \
    2>/dev/null || echo "(scheduler job already exists — use \`update\` to change it)"

gcloud scheduler jobs create http agentcount-weekly-sweep-bsc \
    --project "$PROJECT" --location "$SCHED_LOCATION" \
    --schedule "0 6 * * 2" --time-zone "Etc/UTC" \
    --uri "https://${REGION}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${PROJECT}/jobs/agentcount-sweep-bsc:run" \
    --http-method POST \
    --oauth-service-account-email "$SCHED_SA" \
    2>/dev/null || echo "(bsc scheduler job already exists — use \`update\` to change it)"

# Base on Wednesday, alone.
gcloud scheduler jobs create http agentcount-weekly-sweep-base \
    --project "$PROJECT" --location "$SCHED_LOCATION" \
    --schedule "0 6 * * 3" --time-zone "Etc/UTC" \
    --uri "https://${REGION}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${PROJECT}/jobs/agentcount-sweep-base:run" \
    --http-method POST \
    --oauth-service-account-email "$SCHED_SA" \
    2>/dev/null || echo "(base scheduler job already exists — use \`update\` to change it)"

# The Seller Census on Thursday.
gcloud scheduler jobs create http agentcount-weekly-sellers \
    --project "$PROJECT" --location "$SCHED_LOCATION" \
    --schedule "0 6 * * 4" --time-zone "Etc/UTC" \
    --uri "https://${REGION}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${PROJECT}/jobs/agentcount-sellers:run" \
    --http-method POST \
    --oauth-service-account-email "$SCHED_SA" \
    2>/dev/null || echo "(sellers scheduler job already exists — use \`update\` to change it)"

cat <<'NOTES'

Done. Two things this script does NOT set up, both of which matter:

  ALERTING ON FAILURE. A Cloud Run Job that exits non-zero emits a log entry;
  turn that into an email with a log-based alert policy:

    resource.type="cloud_run_job"
    resource.labels.job_name="agentcount-sweep"
    severity>=ERROR

  ALERTING ON SILENCE is now handled, but needs one thing from you.

  The `heartbeat` binary runs LAST in the weekly job and pings an external
  monitor only once every chain has a finished run that is actually PUBLISHED.
  It stays silent otherwise — including when a sweep succeeded but its upload
  did not, which from a reader's side is the same as no sweep at all.

  For that to alert, set HEARTBEAT_URL before running this script:

    HEARTBEAT_URL=https://... ./scripts/deploy-weekly-sweep.sh

  Any dead-man's-switch service works (healthchecks.io, Better Stack, Cronitor
  — all have free tiers). Create a check with a period of 1 week and a grace
  of 2 days, and use the ping URL it gives you.

  Unset, the job logs a warning and pings nothing — which means a schedule that
  stops firing is STILL invisible. That is the one gap this deployment cannot
  close on its own, because a watchdog hosted on the same infrastructure as the
  thing it watches goes down in the same outage.

NOTES
