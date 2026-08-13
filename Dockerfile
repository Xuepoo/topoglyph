# syntax=docker/dockerfile:1
FROM rust:slim-bookworm AS builder
WORKDIR /usr/src/app

# Install build dependencies (no FFmpeg: video feature is opt-in and
# vectomancy-video 7.1.3 on crates.io links ffmpeg-next 8.1 which is
# incompatible with Debian bookworm. Re-enable when vectomancy-video 7.1.4
# (ffmpeg-next 9) is published and the feature is re-enabled as default.)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    clang \
    libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests and the workspace
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY crates ./crates

# Build the release binary without the video feature
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/src/app/target \
    cargo build --release --bin topoglyph --no-default-features && \
    cp ./target/release/topoglyph /tmp/topoglyph

# Runtime Stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get upgrade -y \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/topoglyph /usr/local/bin/topoglyph

WORKDIR /data

ENTRYPOINT ["topoglyph"]
