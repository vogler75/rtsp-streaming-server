# --- Stage 1: Build ---
FROM rust:1.98.0-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the source code and build the binary
COPY . .
RUN cargo build --release

# --- Stage 2: Runtime ---
FROM debian:bookworm-slim

# Install FFmpeg and runtime dependencies (OpenSSL)
RUN apt-get update && apt-get install -y \
    ffmpeg \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/rtsp-streaming-server /app/rtsp-streaming-server

# Copy static assets if they aren't embedded in the binary
RUN mkdir /app/static
COPY static/ /app/static/

ENTRYPOINT ["/app/rtsp-streaming-server"]
CMD ["--config", "config.json"]