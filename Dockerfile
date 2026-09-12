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
# the OpenSSL headers even though sqlx itself is on rustls.
FROM rust:1-bookworm AS build

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY backend ./backend

# The SPA is served off the filesystem at runtime, not embedded, so the web
# stage is not an input here.
RUN cargo build --release --bin matrix-bot

# ---- Stage 3: runtime -------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd -g 10001 app \
 && useradd -u 10001 -g app -s /usr/sbin/nologin app

WORKDIR /app

COPY --from=build /src/target/release/matrix-bot /usr/local/bin/matrix-bot
COPY --from=web   /src/backend/web/dist          /app/web/dist

# `api/mod.rs` probes ./web/dist first, and `bot/client.rs` hardcodes the crypto
# store at the relative path `data/bot_store` — both resolve against WORKDIR, so
# /app must stay the working directory.
RUN mkdir -p /app/data && chown -R app:app /app
USER app

ENV API_HOST=0.0.0.0 \
    API_PORT=8080 \
    DATABASE_URL="sqlite:///app/data/bot_data.db?mode=rwc"

EXPOSE 8080
VOLUME ["/app/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -f http://127.0.0.1:8080/api/v1/health || exit 1

CMD ["matrix-bot"]
