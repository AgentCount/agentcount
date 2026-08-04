# AgentCount API — container image for Cloud Run.
#
# Multi-stage: a full Rust toolchain to build, a minimal Debian to run. The
# runtime image carries the binary and CA certificates and nothing else — no
# compiler, no cargo registry, no source. That is a smaller attack surface and
# a much smaller pull, which matters on a scale-to-zero platform where image
# size is part of cold-start latency.
#
# Note this builds ONLY `-p api`. The sweeper and indexer are operator tools
# run from a workstation against an RPC endpoint; they have no business in a
# public-facing image, and building them would drag in the whole chain stack
# for nothing.

# ── build ────────────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS build
WORKDIR /src

# Dependencies first, in their own layer. Cargo needs the workspace's manifests
# and a stub for every member before it will resolve anything, so the manifests
# are copied and the real sources are not — a change to a .rs file then reuses
# this layer instead of rebuilding every dependency from scratch.
# OpenSSL headers, because `reqwest` resolves to native-tls in this workspace
# and `api` now links it: the spot-check route drives `crates/probe` (an HTTPS
# prober) and `crates/chain` (an RPC client). Until that route existed the API
# spoke only to Postgres and this layer was unnecessary. `Dockerfile.sweep`
# has carried the same two packages for the same reason since it was written.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends pkg-config libssl-dev; \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates/api/Cargo.toml crates/api/
COPY crates/chain/Cargo.toml crates/chain/
COPY crates/checks/Cargo.toml crates/checks/
COPY crates/indexer/Cargo.toml crates/indexer/
COPY crates/probe/Cargo.toml crates/probe/
COPY crates/sweeper/Cargo.toml crates/sweeper/
RUN set -eux; \
    for c in api chain checks indexer probe sweeper; do \
        mkdir -p "crates/$c/src"; \
        echo 'fn main() {}' > "crates/$c/src/main.rs"; \
        echo '' > "crates/$c/src/lib.rs"; \
    done; \
    # The sweeper's build script reads .git to stamp `checker_commit`; there is
    # no .git here, and it is written to tolerate that.
    cargo build --release -p api || true

# Now the real sources.
COPY crates crates
# Cargo caches by mtime; the stubs above are newer than the files just copied,
# so touch the real entry points or the stub build is considered current and
# the binary silently stays a `fn main() {}`.
RUN set -eux; \
    find crates -name '*.rs' -exec touch {} +; \
    cargo build --release -p api; \
    strip target/release/api

# ── runtime ──────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# ca-certificates AND libssl3. The API used to talk to Postgres and nothing
# else, which is what the previous comment here said; the spot-check route
# changed that. It now makes outbound HTTPS requests to agent-declared hosts
# and JSON-RPC calls to chain endpoints, so it needs both the trust store and
# the OpenSSL runtime the binary is dynamically linked against.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates libssl3; \
    rm -rf /var/lib/apt/lists/*

# Run as a non-root user. Cloud Run does not require it, but a process that
# never needs to write outside /tmp has no reason to be root, and the cost of
# doing it now is one line.
RUN useradd --system --create-home --shell /usr/sbin/nologin agentcount
USER agentcount

COPY --from=build /src/target/release/api /usr/local/bin/agentcount-api

# Documentation only — Cloud Run injects $PORT and the binary reads it. If the
# platform ever picks something other than 8080, the app follows.
EXPOSE 8080
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/agentcount-api"]
