FROM rust:1.99.1-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY server ./server
COPY shared ./shared
COPY wallet ./wallet

RUN cargo build --locked --release --package server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 northbound

COPY --from=builder /app/target/release/server /usr/local/bin/northbound-server

USER northbound

ENV TRUENORTH_SERVER_BIND=0.0.0.0:3000

EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/northbound-server"]
