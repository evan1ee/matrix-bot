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
# the OpenSSL headers even though sqlx itself is on rustls. cmake is for
# opusic-sys, which compiles a vendored libopus from source at build time —
# the result is statically linked, so the runtime stage needs no libopus package.
FROM rust:1-bookworm AS build

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev cmake \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
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
COPY --from=build /src/target/release/libsherpa-onnx-c-api.so /usr/local/lib/
COPY --from=build /src/target/release/libonnxruntime.so       /usr/local/lib/
RUN ldconfig
COPY --from=web   /src/backend/web/dist          /app/web/dist
COPY docker-entrypoint.sh                        /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# `api/mod.rs` probes ./web/dist first, and `bot/client.rs` hardcodes the crypto
# store at the relative path `data/bot_store` — both resolve against WORKDIR, so
# /app must stay the working directory.
RUN mkdir -p /app/data && chown -R app:app /app

ENV API_HOST=0.0.0.0 \
    API_PORT=8080 \
    DATABASE_URL="sqlite:///app/data/bot_data.db?mode=rwc"

EXPOSE 8080
VOLUME ["/app/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -f http://127.0.0.1:8080/api/v1/health || exit 1

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["matrix-bot"]
