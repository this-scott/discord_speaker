# trying cargo chef build
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# System libs needed to compile deps:
#   openssl-sys -> libssl-dev + pkg-config
#   aws-lc-sys  -> cmake + clang (C build + bindgen)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake clang \
    && rm -rf /var/lib/apt/lists/*
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin discord_spot_speaker

# We do not need the Rust toolchain to run the binary!
FROM debian:trixie-slim AS runtime
WORKDIR /app
# Runtime libs: ca-certificates for TLS, libssl3 for openssl-sys dynamic link
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/discord_spot_speaker /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]
