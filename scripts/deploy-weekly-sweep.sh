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
#   Artifact Registry image   built from Dockerfile.sweep (sweeper + liveness + delta)
#   Cloud Run Job             agentcount-sweep, 24h task timeout, no retries
#   Cloud Scheduler job       Mondays 06:00 UTC, triggering the above
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

echo "==> 1/4 build and push the sweep image"
# Built from Dockerfile.sweep, which is separate from the API's image on
# purpose: the public-facing container must not carry a binary that holds RPC
# credentials and writes to every table.
gcloud builds submit --project "$PROJECT" --region "$REGION" \
    --config=- . <<EOF
steps:
  - name: gcr.io/cloud-builders/docker
    args: ['build', '-f', 'Dockerfile.sweep', '-t', '${IMAGE}', '.']
images: ['${IMAGE}']
EOF

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

echo "==> 3/4 the Cloud Run Job"
gcloud run jobs deploy agentcount-sweep \
    --project "$PROJECT" --region "$REGION" \
    --image "$IMAGE" \
    --set-cloudsql-instances "$INSTANCE" \
    --set-secrets "DATABASE_URL=agentcount-db-url:latest,\
RPC_URL_BASE=rpc-url-base:latest,\
RPC_URL_BSC=rpc-url-bsc:latest,\
RPC_URL_MAINNET=rpc-url-mainnet:latest,\
RPC_URL_CELO=rpc-url-celo:latest" \
    --task-timeout 24h \
    --max-retries 0 \
    --memory 2Gi --cpu 2 \
    --parallelism 1 --tasks 1

echo "==> 4/4 the schedule"
# Monday 06:00 UTC. Weekly rather than daily because the delta's unit is a
# week, and because rung 6's traffic to third-party servers is only defensible
# at this cadence.
gcloud scheduler jobs create http agentcount-weekly-sweep \
    --project "$PROJECT" --location "$REGION" \
    --schedule "0 6 * * 1" --time-zone "Etc/UTC" \
    --uri "https://${REGION}-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/${PROJECT}/jobs/agentcount-sweep:run" \
    --http-method POST \
    --oauth-service-account-email "$(gcloud config get-value account)" \
    2>/dev/null || echo "(scheduler job already exists — use \`update\` to change it)"

cat <<'NOTES'

Done. Two things this script does NOT set up, both of which matter:

  ALERTING ON FAILURE. A Cloud Run Job that exits non-zero emits a log entry;
  turn that into an email with a log-based alert policy:

    resource.type="cloud_run_job"
    resource.labels.job_name="agentcount-sweep"
    severity>=ERROR

  ALERTING ON SILENCE. The worse failure is the one that produces no error at
  all — the Celo scan that sat at 0% CPU for three hours. Migration 0014 makes
  that visible in the data (`runs.status`, `runs.last_progress_at`), and the
  sweeper's own watchdog exits 75 on a stall, which the job surfaces as a
  failure. But NOTHING notices a run that never started, because a scheduler
  job that silently stops firing produces no log line to alert on.

  The check for that is an absence: no finished run for a chain in 8 days.
  It needs somewhere to run that is not this job — a second, tiny scheduled
  check, or an uptime check against an API endpoint that reports it. Until it
  exists, a schedule that stops firing is invisible.

NOTES
