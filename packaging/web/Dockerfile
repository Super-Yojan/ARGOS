FROM rust:1-slim-bookworm AS rust
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY spikes/unity-dashboard/native/argos-unity-bridge ./spikes/unity-dashboard/native/argos-unity-bridge
RUN cargo build --release --locked -p argos-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates sqlite3 && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home argos && mkdir -p /data /app/web && chown argos /data /app/web
COPY --from=rust /src/target/release/argos-server /usr/local/bin/argos-server
USER argos
ENV ARGOS_WEB_DIR=/app/web ARGOS_DATABASE=sqlite:///data/argos.db ARGOS_ENV=production ARGOS_DEV_AUTH=false
VOLUME /data
ENTRYPOINT ["argos-server"]
