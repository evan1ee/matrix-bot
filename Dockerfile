# syntax=docker/dockerfile:1

# ---- Stage 1: build the SPA -------------------------------------------------
# vite.config.ts writes to `../backend/web/dist`, so the source tree must keep
# its `frontend/` + `backend/` sibling layout inside the builder.
FROM node:22-bookworm-slim AS web

WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci

COPY frontend/ ./
RUN npm run build
# -> /src/backend/web/dist

# ---- Stage 2: build the bot -------------------------------------------------
# reqwest pulls in native-tls (openssl-sys is in Cargo.lock), so the build needs
# the OpenSSL headers even though sqlx itself is on rustls. No cmake: the bot
# links no native ML or codec library; those live in the inference image.
FROM rust:1-bookworm AS build

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

# Compile-checked sqlx queries read the committed offline cache instead of a
# live database at build time (see .cargo/config.toml) -- the builder has no
# DATABASE_URL and must not need one.
ENV SQLX_OFFLINE=true

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY .sqlx ./.sqlx
COPY backend/Cargo.toml ./backend/Cargo.toml
COPY inference/Cargo.toml ./inference/Cargo.toml

# Cargo loads every workspace member's manifest before building anything, even
# with `default-members` restricting the bare `cargo build`, so `inference`'s
# manifest and a dummy source file both have to be here even though this image
# never builds that crate's binary — see Dockerfile.inference for the same
# pattern in the other direction.
#
# Pre-build dependency layer with dummy source files to cache crates across source edits
RUN mkdir -p backend/src backend/src/bin inference/src && \
    echo "pub fn dummy() {}" > backend/src/lib.rs && \
    echo "fn main() {}" > backend/src/main.rs && \
    echo "fn main() {}" > backend/src/bin/generate_openapi.rs && \
    echo "fn main() {}" > inference/src/main.rs && \
    mkdir -p backend/web/dist && \
    touch backend/web/dist/index.html && \
    cargo build --release --bin matrix-bot && \
    rm -rf backend/src target/release/deps/matrix_bot* target/release/matrix-bot*
# inference/src stays as the dummy: this image never builds that crate's
# binary, nothing re-populates it (there is no `COPY inference ./inference`
# below), and removing it here would make the final `cargo build` below fail
# to load the workspace, exactly as the manifest-only version of this bug did.

COPY backend ./backend

# The SPA is served off the filesystem at runtime, not embedded, so the web
# stage is not an input here.
RUN cargo build --release --bin matrix-bot

# ---- Stage 3: runtime -------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 curl gosu \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd -g 10001 app \
 && useradd -u 10001 -g app -s /usr/sbin/nologin app

WORKDIR /app

COPY --from=build /src/target/release/matrix-bot /usr/local/bin/matrix-bot
COPY --from=web   /src/backend/web/dist          /app/web/dist
COPY docker-entrypoint.sh                        /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# `api/mod.rs` probes ./web/dist first, and `bot/client.rs` hardcodes the crypto
# store at the relative path `data/bot_store` — both resolve against WORKDIR, so
# /app must stay the working directory.
RUN mkdir -p /app/data && chown -R app:app /app

ENV API_HOST=0.0.0.0 \
    API_PORT=8080

# DATABASE_URL has no default here -- it must be supplied at runtime (e.g. by
# compose) and must be a postgres:// URL; PostgreSQL is the only application
# database. /app/data holds only matrix-sdk's own SQLite crypto store.
EXPOSE 8080
VOLUME ["/app/data"]

# Liveness, not readiness: a container restart cannot fix a homeserver outage
# or a lost database connection, so only "is the process alive" should drive a
# restart. `/health/ready` (database + Matrix sync freshness) is for a load
# balancer/orchestrator to route traffic on.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -f http://127.0.0.1:8080/health/live || exit 1

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["matrix-bot"]
