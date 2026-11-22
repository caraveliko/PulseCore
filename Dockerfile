# syntax=docker/dockerfile:1

# ==============================================================================
# Etapa 1: Planificación de dependencias con cargo-chef
# ==============================================================================
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

# ==============================================================================
# Etapa 2: Compilación en caché de dependencias y binarios estándar
# ==============================================================================
FROM chef AS builder

# Crear carpeta web mínima para que rust-embed valide la ruta durante cargo chef cook.
RUN mkdir -p /app/web
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p mule-daemon -p mule-cli

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY web ./web

# Compilación estándar de producción (prohibido habilitar lab-tools).
RUN cargo build --release -p mule-daemon -p mule-cli

# ==============================================================================
# Etapa 3: Imagen final mínima sobre Debian Bookworm Slim (glibc + jemalloc)
# ==============================================================================
FROM debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.title="Pulse Core (eMule-Rust)" \
      org.opencontainers.image.description="Daemon P2P eD2K, Kad DHT y BitTorrent con WebUI embebida y automatización Servarr" \
      org.opencontainers.image.source="https://github.com/caraveliko/PulseCore" \
      org.opencontainers.image.licenses="GPL-2.0-or-later"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata gosu curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 pulsecore \
    && useradd --uid 1000 --gid 1000 --home-dir /config --shell /usr/sbin/nologin pulsecore \
    && mkdir -p /config /data/downloads /data/media/Movies /data/media/Series \
    && chown -R 1000:1000 /config /data

COPY --from=builder /app/target/release/mule-daemon /usr/local/bin/mule-daemon
COPY --from=builder /app/target/release/mule-cli /usr/local/bin/mule-cli
COPY docker/entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

ENV EMULERUST_WORKDIR=/config \
    EMULERUST_DOWNLOADS_DIR=/data/downloads \
    EMULERUST_NAS_PATH=/data/media \
    EMULERUST_MOVIES_DIR=/data/media/Movies \
    EMULERUST_SERIES_DIR=/data/media/Series \
    EMULERUST_UPDATE_CHECK_ENABLED=false \
    PULSE_DOCKER=1 \
    MULE_NO_TRAY=1 \
    MULE_BIND_ADDR=0.0.0.0:4660 \
    MULE_ED2K_PORT=39735 \
    MULE_KAD_PORT=39737 \
    MULE_UPNP=false \
    PUID=1000 \
    PGID=1000 \
    RUST_LOG=info

EXPOSE 4660/tcp 39735/tcp 39737/udp
VOLUME ["/config", "/data"]
STOPSIGNAL SIGTERM

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:4660/health || exit 1

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
