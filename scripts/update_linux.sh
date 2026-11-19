#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Copyright (C) 2026 Laurent Geynet "Loulach"
#
# Script de compilación, sincronización systemd, rollback y despliegue de Pulse Core (`mule-daemon`)
# en Linux y Raspberry Pi (`aarch64`). Permite compilar desde la raíz del repositorio y desplegar
# vía subida HTTP autenticada (`/api/system/upload-update`) o sustitución atómica por SSH.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SERVICE_TEMPLATE="${REPO_ROOT}/scripts/mule-daemon.service"

MODE=""
HOST="127.0.0.1"
PORT="4660"
SSH_USER="caraveliko"
BINARY_PATH=""
AUTH_TOKEN="${MULE_AUTH_TOKEN:-}"
USE_SSH=false
AUTO_RESTART=true
INSTALL_SERVICE=false
DO_ROLLBACK=false

mostrar_ayuda() {
  cat <<'EOF'
Uso: scripts/update_linux.sh [MODO] [OPCIONES]

Modos de operación (elegir uno):
  --local               Compila mule-daemon para el host Linux actual y actualiza 127.0.0.1
  --rpi                 Compila de forma cruzada (aarch64-unknown-linux-gnu) y actualiza Raspberry Pi
  --binary <ruta>       Usa un binario ya compilado sin invocar cargo
  --rollback            Revierte al binario previo (.old) vía HTTP/RPC o SSH (--ssh)
  --install-service     Sincroniza solo la unidad systemd de usuario (~/.config/systemd/user)

Opciones:
  --host <ip_o_host>    Dirección del demonio (por defecto: 127.0.0.1 en --local, 192.168.1.166 en --rpi)
  --port <puerto>       Puerto HTTP del demonio (por defecto: 4660)
  --user <usuario>      Usuario SSH en destino (por defecto: caraveliko)
  --token <token>       Token Bearer de autenticación (si se omite, lee ~/.mule/auth.token o MULE_AUTH_TOKEN)
  --ssh                 Despliegue/rollback por SSH (crea .old, sincroniza servicio systemd y reinicia)
  --no-restart          Instala o revierte sin reiniciar automáticamente el demonio
  -h, --help            Muestra esta ayuda y sale
EOF
}

HOST_EXPLICITO=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --local)
      MODE="local"
      shift
      ;;
    --rpi)
      MODE="rpi"
      if [[ "${HOST_EXPLICITO}" == false ]]; then
        HOST="192.168.1.166"
      fi
      shift
      ;;
    --binary)
      MODE="binary"
      BINARY_PATH="${2:-}"
      if [[ -z "${BINARY_PATH}" ]]; then
        echo "Error: --binary requiere una ruta válida" >&2
        exit 1
      fi
      shift 2
      ;;
    --install-service)
      INSTALL_SERVICE=true
      shift
      ;;
    --rollback)
      DO_ROLLBACK=true
      shift
      ;;
    --host)
      HOST="${2:-}"
      HOST_EXPLICITO=true
      shift 2
      ;;
    --port)
      PORT="${2:-}"
      shift 2
      ;;
    --user)
      SSH_USER="${2:-}"
      shift 2
      ;;
    --token)
      AUTH_TOKEN="${2:-}"
      shift 2
      ;;
    --ssh)
      USE_SSH=true
      shift
      ;;
    --no-restart)
      AUTO_RESTART=false
      shift
      ;;
    -h|--help)
      mostrar_ayuda
      exit 0
      ;;
    *)
      echo "Opción desconocida: $1" >&2
      mostrar_ayuda
      exit 1
      ;;
  esac
done

es_host_local() {
  [[ "${HOST}" == "127.0.0.1" || "${HOST}" == "localhost" ]]
}

resolver_token() {
  if [[ -n "${AUTH_TOKEN}" ]]; then
    return 0
  fi
  if es_host_local; then
    if [[ -f "${HOME}/.mule/auth.token" ]]; then
      AUTH_TOKEN="$(tr -d '\r\n' < "${HOME}/.mule/auth.token")"
    elif [[ -f "${HOME}/.mule/token" ]]; then
      AUTH_TOKEN="$(tr -d '\r\n' < "${HOME}/.mule/token")"
    fi
  elif command -v ssh >/dev/null 2>&1; then
    echo "==> Obteniendo token de sesión desde ${SSH_USER}@${HOST}:~/.mule/auth.token..."
    AUTH_TOKEN="$(ssh -o BatchMode=yes -o ConnectTimeout=5 "${SSH_USER}@${HOST}" \
      'cat ~/.mule/auth.token 2>/dev/null || cat ~/.mule/token 2>/dev/null || true' | tr -d '\r\n')"
  fi
}

instalar_unidad_systemd() {
  if [[ ! -f "${SERVICE_TEMPLATE}" ]]; then
    echo "Error: no se encontró la plantilla ${SERVICE_TEMPLATE}" >&2
    exit 1
  fi
  if es_host_local && [[ "${USE_SSH}" == false ]]; then
    echo "==> Instalando unidad systemd de usuario en ${HOME}/.config/systemd/user/mule-daemon.service..."
    mkdir -p "${HOME}/.local/bin" "${HOME}/.config/systemd/user" "${HOME}/.mule" "${HOME}/Downloads"
    cp "${SERVICE_TEMPLATE}" "${HOME}/.config/systemd/user/mule-daemon.service"
    systemctl --user daemon-reload
    loginctl enable-linger "${USER}" 2>/dev/null || true
    echo "==> Unidad systemd local sincronizada (daemon-reload completado)."
  else
    local destino="${SSH_USER}@${HOST}"
    echo "==> Sincronizando unidad systemd de usuario en ${destino}..."
    ssh "${destino}" 'mkdir -p ~/.local/bin ~/.config/systemd/user ~/.mule ~/Downloads'
    scp "${SERVICE_TEMPLATE}" "${destino}:~/.config/systemd/user/mule-daemon.service"
    ssh "${destino}" 'systemctl --user daemon-reload && (loginctl enable-linger "$USER" 2>/dev/null || true)'
    echo "==> Unidad systemd remota sincronizada en ${destino}."
  fi
}

esperar_salud() {
  local url="http://${HOST}:${PORT}/health"
  echo "==> Esperando a que Pulse Core vuelva a responder en ${url}..."
  sleep 1.5
  local intentos=0
  local max_intentos=30
  while [[ ${intentos} -lt ${max_intentos} ]]; do
    if resp="$(curl -fsS --max-time 2 "${url}" 2>/dev/null)"; then
      echo "==> Pulse Core en línea tras reinicio: ${resp}"
      return 0
    fi
    intentos=$((intentos + 1))
    sleep 1
  done
  echo "Aviso: se agotó la espera de ${max_intentos}s consultando ${url}." >&2
  return 1
}

if [[ "${DO_ROLLBACK}" == true ]]; then
  if [[ "${USE_SSH}" == true ]]; then
    destino="${SSH_USER}@${HOST}"
    echo "==> Ejecutando rollback por SSH sobre ${destino}..."
    ssh "${destino}" '
      if [[ ! -f ~/.local/bin/mule-daemon.old ]]; then
        echo "Error: no existe ~/.local/bin/mule-daemon.old en destino" >&2
        exit 1
      fi
      chmod 755 ~/.local/bin/mule-daemon.old
      mv -f ~/.local/bin/mule-daemon.old ~/.local/bin/mule-daemon
    '
    if [[ "${AUTO_RESTART}" == true ]]; then
      ssh "${destino}" 'systemctl --user restart mule-daemon.service'
      esperar_salud || true
    fi
    exit 0
  fi

  resolver_token
  RPC_URL="http://${HOST}:${PORT}/api/rpc"
  echo "==> Solicitando rollback vía JSON-RPC en ${RPC_URL}..."
  RPC_HEADERS=(-H "Content-Type: application/json")
  if [[ -n "${AUTH_TOKEN}" ]]; then
    RPC_HEADERS+=(-H "Authorization: Bearer ${AUTH_TOKEN}")
  fi
  HTTP_BODY_FILE="$(mktemp)"
  trap 'rm -f "${HTTP_BODY_FILE}"' EXIT
  HTTP_CODE="$(curl -sS -o "${HTTP_BODY_FILE}" -w "%{http_code}" \
    -X POST "${RPC_HEADERS[@]}" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.rollbackUpdate\",\"params\":{\"autoRestart\":${AUTO_RESTART}}}" \
    "${RPC_URL}" || true)"
  echo "==> Respuesta (${HTTP_CODE}): $(cat "${HTTP_BODY_FILE}")"
  if [[ "${HTTP_CODE}" != "200" ]] || grep -q '"error"' "${HTTP_BODY_FILE}"; then
    exit 1
  fi
  if [[ "${AUTO_RESTART}" == true ]]; then
    esperar_salud
  fi
  exit 0
fi

if [[ "${INSTALL_SERVICE}" == true ]]; then
  instalar_unidad_systemd
  if [[ -z "${MODE}" ]]; then
    exit 0
  fi
fi

if [[ -z "${MODE}" ]]; then
  echo "Error: debes indicar --local, --rpi, --binary <ruta>, --rollback o --install-service." >&2
  mostrar_ayuda
  exit 1
fi

# Ejecutar siempre cargo desde la raíz del repositorio para respetar .cargo/config.toml
cd "${REPO_ROOT}"

if [[ "${MODE}" == "local" ]]; then
  echo "==> Compilando mule-daemon (release local) desde ${REPO_ROOT}..."
  cargo build --release -p mule-daemon
  BINARY_PATH="${REPO_ROOT}/target/release/mule-daemon"
elif [[ "${MODE}" == "rpi" ]]; then
  echo "==> Compilando de forma cruzada mule-daemon (aarch64-unknown-linux-gnu) desde ${REPO_ROOT}..."
  cargo build --release --target aarch64-unknown-linux-gnu -p mule-daemon
  BINARY_PATH="${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release/mule-daemon"
fi

if [[ ! -f "${BINARY_PATH}" ]]; then
  echo "Error: no existe el binario esperado en ${BINARY_PATH}" >&2
  exit 1
fi

SHA256="$(sha256sum "${BINARY_PATH}" | awk '{print $1}')"
FILENAME="$(basename "${BINARY_PATH}")"
SIZE_BYTES="$(wc -c < "${BINARY_PATH}")"

echo "==> Binario listo: ${BINARY_PATH}"
echo "    Tamaño : ${SIZE_BYTES} bytes"
echo "    SHA-256: ${SHA256}"

desplegar_ssh() {
  local destino="${SSH_USER}@${HOST}"
  echo "==> Desplegando de forma atómica por SSH en ${destino}:~/.local/bin/mule-daemon..."
  ssh "${destino}" 'mkdir -p ~/.local/bin ~/.config/systemd/user'
  if ssh "${destino}" '[[ ! -f ~/.config/systemd/user/mule-daemon.service ]] || ! grep -q "%h/.local/bin" ~/.config/systemd/user/mule-daemon.service'; then
    echo "==> Detectada unidad systemd ausente o sin ReadWritePaths=%h/.local/bin; sincronizando plantilla..."
    scp "${SERVICE_TEMPLATE}" "${destino}:~/.config/systemd/user/mule-daemon.service"
    ssh "${destino}" 'systemctl --user daemon-reload && (loginctl enable-linger "$USER" 2>/dev/null || true)'
  fi
  scp "${BINARY_PATH}" "${destino}:~/.local/bin/mule-daemon.update"
  ssh "${destino}" '
    if [[ -f ~/.local/bin/mule-daemon ]]; then
      ln -f ~/.local/bin/mule-daemon ~/.local/bin/mule-daemon.old 2>/dev/null || cp -f ~/.local/bin/mule-daemon ~/.local/bin/mule-daemon.old
      chmod 755 ~/.local/bin/mule-daemon.old
    fi
    chmod 755 ~/.local/bin/mule-daemon.update
    mv -f ~/.local/bin/mule-daemon.update ~/.local/bin/mule-daemon
  '
  if [[ "${AUTO_RESTART}" == true ]]; then
    echo "==> Reiniciando unidad systemd de usuario en ${destino}..."
    ssh "${destino}" 'systemctl --user restart mule-daemon.service'
  fi
}

if [[ "${USE_SSH}" == true ]]; then
  desplegar_ssh
  if [[ "${AUTO_RESTART}" == true ]]; then
    esperar_salud || true
  fi
  exit 0
fi

resolver_token

UPLOAD_URL="http://${HOST}:${PORT}/api/system/upload-update"
echo "==> Subiendo actualización a ${UPLOAD_URL}..."

CURL_HEADERS=(
  -H "Content-Type: application/octet-stream"
  -H "X-Update-Filename: ${FILENAME}"
  -H "X-Update-Sha256: ${SHA256}"
  -H "X-Auto-Restart: ${AUTO_RESTART}"
)
if [[ -n "${AUTH_TOKEN}" ]]; then
  CURL_HEADERS+=(-H "Authorization: Bearer ${AUTH_TOKEN}")
fi

HTTP_BODY_FILE="$(mktemp)"
trap 'rm -f "${HTTP_BODY_FILE}"' EXIT

HTTP_CODE="$(curl -sS -o "${HTTP_BODY_FILE}" -w "%{http_code}" \
  -X POST "${CURL_HEADERS[@]}" \
  --data-binary "@${BINARY_PATH}" \
  "${UPLOAD_URL}" || true)"

if [[ "${HTTP_CODE}" == "200" ]]; then
  echo "==> Respuesta del servidor (HTTP 200): $(cat "${HTTP_BODY_FILE}")"
  if [[ "${AUTO_RESTART}" == true ]]; then
    esperar_salud
  fi
  exit 0
fi

echo "Aviso: la subida HTTP devolvió código ${HTTP_CODE:-desconocido}: $(cat "${HTTP_BODY_FILE}")" >&2
if ! es_host_local; then
  echo "Sugerencia: si el nodo aún no tiene este endpoint o está detenido, ejecuta con --ssh." >&2
fi
exit 1
