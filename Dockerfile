# syntax=docker/dockerfile:1
FROM rust:slim-bookworm AS builder
WORKDIR /usr/src/app

# Install build dependencies for FFmpeg (topoglyph-video, native-only —
# see topoglyph-docs/TODO.md 0.5.0 and the `video` cargo feature)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    clang \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libswresample-dev \
    libavdevice-dev \
    libavfilter-dev \
    libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests and the workspace (topoglyph is a single
# `crates/*` workspace, unlike vectomancy's split cli/text/video top-level
# dirs — everything lives under crates/topoglyph-*).
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY crates ./crates

# Build the release binary for the CLI
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/src/app/target \
    cargo build --release --bin topoglyph && \
    cp ./target/release/topoglyph /tmp/topoglyph

# Runtime Stage
FROM debian:bookworm-slim

# Install runtime dependencies for FFmpeg libs
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libswresample-dev \
    libasound2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/topoglyph /usr/local/bin/topoglyph

# Set default working directory for external data mounts
WORKDIR /data

ENTRYPOINT ["topoglyph"]
