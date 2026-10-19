# syntax=docker/dockerfile:1.7
# ==============================================================================
# Stage 1: Chef - Dependency Caching Layer
# Multi-arch: supports linux/amd64 and linux/arm64 (Graviton, Apple Silicon)
# ==============================================================================
FROM --platform=$BUILDPLATFORM lukemathwalker/cargo-chef:latest-rust-1.99-bookworm@sha256:93aa3e146e39f2841bb9e271ab7a3b771e518721d930771c4384e1820d4fc9f8 AS chef
WORKDIR /app
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
      build-essential cmake pkg-config libssl-dev libsasl2-dev libsasl2-modules libcurl4-openssl-dev && \
    rm -rf /var/lib/apt/lists/*

# ==============================================================================
# Stage 2: Planner - Generate recipe.json for dependency caching
# ==============================================================================
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ==============================================================================
# Stage 3: Builder - Build dependencies (cached) then application
# TARGETPLATFORM / TARGETARCH are injected by docker buildx automatically.
# ==============================================================================
FROM chef AS builder

ARG TARGETPLATFORM
ARG TARGETARCH
ARG BUILDPLATFORM

# Install cross-compilation toolchains when building for arm64 on amd64 host.
RUN if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
      dpkg --add-architecture arm64 && \
      apt-get update -qq && \
      apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        libc6-dev-arm64-cross qemu-user-static \
        libssl-dev:arm64 libsasl2-dev:arm64 libsasl2-modules:arm64 libcurl4-openssl-dev:arm64 && \
      rustup target add aarch64-unknown-linux-gnu; \
    fi

# Set Cargo target based on TARGETARCH.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
ENV PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig
ENV PKG_CONFIG_ALLOW_CROSS=1

# Copy the recipe and build dependencies first (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/app/target \
  set -eu; \
  if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
    cargo chef cook --release --locked --target aarch64-unknown-linux-gnu --recipe-path recipe.json; \
  else \
    cargo chef cook --release --locked --recipe-path recipe.json; \
  fi

# Now copy source and build binaries in a single step to share
# the dependency cache layer and avoid redundant recompilation.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/app/target \
  set -eu; \
  if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
    cargo build --release --locked --target aarch64-unknown-linux-gnu \
      --bin stellar-operator \
      --bin kubectl-stellar \
      --bin stellar-sidecar \
      --bin stellar-watcher \
      --bin stellar-fork-detector \
      --bin soroban-cache-proxy && \
    mkdir -p target/release && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-operator target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/kubectl-stellar target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-sidecar target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-watcher target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-fork-detector target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/soroban-cache-proxy target/release/; \
  else \
    cargo build --release --locked \
      --bin stellar-operator \
      --bin kubectl-stellar \
      --bin stellar-sidecar \
      --bin stellar-watcher \
      --bin stellar-fork-detector \
      --bin soroban-cache-proxy; \
  fi; \
  mkdir -p /app/bin; \
  cp target/release/stellar-operator target/release/kubectl-stellar \
    target/release/stellar-sidecar target/release/stellar-watcher \
    target/release/stellar-fork-detector target/release/soroban-cache-proxy /app/bin/

# Strip binaries to reduce image size
RUN if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
      aarch64-linux-gnu-strip /app/bin/*; \
    else strip /app/bin/*; fi

# Carry only linked native libraries missing from distroless (Kafka TLS/SASL,
# curl, and their dependencies). Keep distroless's patched libc and C++ runtime.
RUN set -eu; \
    mkdir -p /app/runtime-libs; \
    if [ "$TARGETARCH" = "arm64" ]; then sasl_dir=/usr/lib/aarch64-linux-gnu/sasl2; \
    else sasl_dir=/usr/lib/x86_64-linux-gnu/sasl2; fi; \
    if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
      for binary in /app/bin/* "$sasl_dir"/*.so; do \
        qemu-aarch64-static /lib/ld-linux-aarch64.so.1 --list "$binary"; \
      done > /app/linked-libraries; \
    else ldd /app/bin/* "$sasl_dir"/*.so > /app/linked-libraries; fi; \
    if grep -q 'not found' /app/linked-libraries; then cat /app/linked-libraries; exit 1; fi; \
    awk '$2 == "=>" && $3 ~ /^\// {print $3}' /app/linked-libraries | sort -u > /app/library-paths; \
    while IFS= read -r library; do \
      case "$(basename "$library")" in \
        libc.so.*|libm.so.*|libpthread.so.*|libdl.so.*|librt.so.*|libresolv.so.*|libstdc++.so.*|libgcc_s.so.*) continue ;; \
      esac; \
      cp -L --parents "$library" /app/runtime-libs; \
    done < /app/library-paths; \
    cp -a --parents "$sasl_dir" /app/runtime-libs

# ==============================================================================
# Native HTTP probe, built for the runtime architecture independently of Rust.
# Local packaging can copy this stage without triggering application compilation.
# ==============================================================================
FROM --platform=$TARGETPLATFORM lukemathwalker/cargo-chef:latest-rust-1.99-bookworm@sha256:93aa3e146e39f2841bb9e271ab7a3b771e518721d930771c4384e1820d4fc9f8 AS health-probe
RUN set -eu; apt-get update -qq && apt-get install -y --no-install-recommends curl && \
    mkdir -p /probe/usr/bin && cp /usr/bin/curl /probe/usr/bin/curl && \
    ldd /usr/bin/curl > /tmp/probe-libraries && \
    awk '$2 == "=>" && $3 ~ /^\// {print $3}' /tmp/probe-libraries | sort -u > /tmp/probe-paths; \
    if grep -q 'not found' /tmp/probe-libraries; then cat /tmp/probe-libraries; exit 1; fi; \
    while IFS= read -r library; do \
      case "$(basename "$library")" in \
        libc.so.*|libm.so.*|libpthread.so.*|libdl.so.*|librt.so.*|libresolv.so.*|libstdc++.so.*|libgcc_s.so.*) continue ;; \
      esac; \
      cp -L --parents "$library" /probe; \
    done < /tmp/probe-paths

# ==============================================================================
# Stage 4: Local Binaries - Fast local packaging from host build artifacts
# ==============================================================================
FROM scratch AS local-binaries
COPY target/release/stellar-operator /stellar-operator
COPY target/release/kubectl-stellar /kubectl-stellar
COPY target/release/soroban-cache-proxy /soroban-cache-proxy

# ==============================================================================
# Stage 5: Runtime Local - Minimal image for local dev (no container recompile)
# ==============================================================================
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime-local
COPY --from=health-probe /probe/ /

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy prebuilt local binaries
COPY --from=local-binaries /stellar-operator /stellar-operator
COPY --from=local-binaries /kubectl-stellar /kubectl-stellar
COPY --from=local-binaries /soroban-cache-proxy /soroban-cache-proxy

# Run as non-root user (UID 65532 is the nonroot user in distroless)
USER nonroot:nonroot

# Expose metrics and REST API ports
EXPOSE 8080 9090

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/usr/bin/curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8080/healthz"]

ENTRYPOINT ["/stellar-operator"]

# ==============================================================================
# Stage 6: Runtime - Minimal distroless image (~15-20MB total)
# ==============================================================================
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime

COPY --from=builder /app/runtime-libs/ /
COPY --from=health-probe /probe/ /

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy stripped binaries
COPY --from=builder /app/bin/stellar-operator /stellar-operator
COPY --from=builder /app/bin/kubectl-stellar /kubectl-stellar
COPY --from=builder /app/bin/stellar-sidecar /stellar-sidecar
COPY --from=builder /app/bin/stellar-watcher /stellar-watcher
COPY --from=builder /app/bin/stellar-fork-detector /stellar-fork-detector
COPY --from=builder /app/bin/soroban-cache-proxy /soroban-cache-proxy

# Run as non-root user (UID 65532 is the nonroot user in distroless)
USER nonroot:nonroot

# Expose metrics and REST API ports
EXPOSE 8080 9090

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/usr/bin/curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8080/healthz"]

ENTRYPOINT ["/stellar-operator"]
