# Multi-stage build of the ObserveCraft image (build context: the repository root):
#   observecraft-server (API + the built web app), oc-intake (telemetry gateway), oc-tracegen (demo data).
# SAML signature verification links libxmlsec1 (OpenSSL backend); bindgen needs libclang at build time.

FROM node:20.18.0-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN VITE_API_MODE=live npm run build

FROM rust:1.99-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libclang-dev libxml2-dev libxmlsec1-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY server/ .
RUN cargo build --release --bin observecraft-server --bin oc-intake --bin oc-tracegen

FROM debian:bookworm-slim
# upgrade: security fixes released after the base image was built.
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends ca-certificates libxml2 libxslt1.1 libxmlsec1 libxmlsec1-openssl libssl3 libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 observecraft \
    && mkdir -p /var/lib/observecraft && chown observecraft /var/lib/observecraft
COPY --from=build /src/target/release/observecraft-server /src/target/release/oc-intake /src/target/release/oc-tracegen /usr/local/bin/
COPY --from=web /web/dist /usr/share/observecraft/web
# The API serves the web app; the archive cache lives in the data directory.
ENV OC_WEB_DIST=/usr/share/observecraft/web OC_DATA_DIR=/var/lib/observecraft
# Numeric, so Kubernetes can verify runAsNonRoot.
USER 10001:10001
VOLUME /var/lib/observecraft
EXPOSE 8080
ENTRYPOINT ["observecraft-server"]
