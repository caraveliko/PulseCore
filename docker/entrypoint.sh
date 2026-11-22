#!/bin/sh
# Punto de entrada del contenedor Docker para Pulse Core (mule-daemon).
set -eu

PUID="${PUID:-1000}"
PGID="${PGID:-1000}"
MULE_BIND_ADDR="${MULE_BIND_ADDR:-0.0.0.0:4660}"
MULE_ED2K_PORT="${MULE_ED2K_PORT:-39735}"
MULE_KAD_PORT="${MULE_KAD_PORT:-39737}"
MULE_UPNP="${MULE_UPNP:-false}"

WORKDIR="${EMULERUST_WORKDIR:-/config}"
DOWNLOADS_DIR="${EMULERUST_DOWNLOADS_DIR:-/data/downloads}"
NAS_PATH="${EMULERUST_NAS_PATH:-/data/media}"
MOVIES_DIR="${EMULERUST_MOVIES_DIR:-/data/media/Movies}"
SERIES_DIR="${EMULERUST_SERIES_DIR:-/data/media/Series}"

# Crear directorios base si los volúmenes están vacíos (sin fallar en montajes de solo lectura).
mkdir -p "$WORKDIR" "$DOWNLOADS_DIR" "$NAS_PATH" "$MOVIES_DIR" "$SERIES_DIR" 2>/dev/null || true

set -- \
    --no-tray \
    --bind "$MULE_BIND_ADDR" \
    --ed2k-port "$MULE_ED2K_PORT" \
    --kad-port "$MULE_KAD_PORT" \
    --upnp "$MULE_UPNP" \
    "$@"

if [ "$(id -u)" = "0" ]; then
    # Ajustar permisos de /config y únicamente de las carpetas raíz de /data para no recorrer bibliotecas grandes.
    chown -R "${PUID}:${PGID}" "$WORKDIR" 2>/dev/null || true
    chown "${PUID}:${PGID}" /data "$DOWNLOADS_DIR" "$NAS_PATH" "$MOVIES_DIR" "$SERIES_DIR" 2>/dev/null || true
    exec gosu "${PUID}:${PGID}" /usr/local/bin/mule-daemon "$@"
fi

exec /usr/local/bin/mule-daemon "$@"
