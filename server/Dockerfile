# Multi-stage build for observecraft-server (and oc-tracegen, the synthetic trace generator).
# SAML signature verification links libxmlsec1 (OpenSSL backend); bindgen needs libclang at build time.
FROM rust:1.89-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libclang-dev libxml2-dev libxmlsec1-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --bin observecraft-server --bin oc-tracegen

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libxml2 libxslt1.1 libxmlsec1 libxmlsec1-openssl libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 observecraft
COPY --from=build /src/target/release/observecraft-server /src/target/release/oc-tracegen /usr/local/bin/
USER observecraft
EXPOSE 8080
ENTRYPOINT ["observecraft-server"]
