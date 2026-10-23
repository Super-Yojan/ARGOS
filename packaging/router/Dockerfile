FROM rust:1.90-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .github/scripts/checkout-observability.sh /tmp/checkout-observability.sh
RUN bash /tmp/checkout-observability.sh
COPY crates ./crates
COPY tests ./tests
COPY spikes/unity-dashboard/native/argos-unity-bridge ./spikes/unity-dashboard/native/argos-unity-bridge
RUN cargo build --locked --release -p argos-zenoh --bin argos-router
FROM debian:bookworm-slim
COPY --from=build /src/target/release/argos-router /usr/local/bin/argos-router
USER 65534:65534
EXPOSE 7448
ENTRYPOINT ["argos-router"]
