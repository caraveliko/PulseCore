# Registro de Decisión Arquitectónica (ADR): Sistema de Actualización Automática Integrado

- **Estado**: Aprobado
- **Fecha**: 2026-11-17
- **Ámbito**: `mule-daemon`, `mule-cli`, WebUI y bandeja de Windows (`tray.rs`)
- **Propietario**: Equipo del núcleo de eMuleRust

---

## 1. Contexto y problemática

eMuleRust se ejecuta en diversos entornos y arquitecturas:
1. **Escritorio Windows (x64)**: Ejecución como aplicación interactiva con icono en la bandeja del sistema (`src/tray.rs`) y WebUI embebida.
2. **Servidores y NAS Linux (x86_64 y aarch64 / Raspberry Pi)**: Ejecución desatendida (*headless*) gestionada a través de unidades de servicio de usuario de `systemd` (`scripts/mule-daemon.service`).

Las actualizaciones plantean retos críticos:
- **Bloqueo de ejecutables en Windows**: El sistema de archivos NTFS/Windows prohíbe escribir o truncar un archivo `.exe` que se encuentra en ejecución (`ERROR_ACCESS_DENIED`), aunque sí consiente su renombrado dentro del mismo volumen.
- **Integridad de datos P2P y SQLite**: Interrumpir abruptamente el proceso puede provocar corrupción de archivos temporales dispersos (`.part`), inconsistencia en los metadatos `.met` o transacciones abiertas en SQLite (`media_library.db`, `downloads.db`).
- **Seguridad y autenticidad del binario**: La descarga y ejecución de código ajeno es un vector crítico de ataque. Se requiere verificación criptográfica asimétrica inmutable (Ed25519) y contención frente a ataques SSRF.

---

## 2. Decisión adoptada: Sistema nativo integrado (*In-App Staging + Atomic Self-Replace*)

Se adopta una arquitectura en dos fases desacopladas, integrada directamente en el demonio (`mule-daemon`), sin requerir instaladores externos ni procesos residentes adicionales:

```
[Manifiesto Remoto HTTPS]
          │
          ▼  (Paso 1: Consulta periódica / manual)
[Servicio Updater en mule-daemon]
          │
          ├─── Valida versión y canal (stable / beta)
          │
          ▼  (Paso 2: Descarga acotada a staging)
[Directorio staging: mule-daemon.new]
          │
          ├─── Verifica SHA-256
          ├─── Verifica firma Ed25519 con clave pública maestra incrustada
          │
          ▼  (Paso 3: Notificación a UI y solicitud de aplicación)
[WebUI / Bandeja Windows / JSON-RPC]
          │
          ▼  (Paso 4: Solicitud de reinicio ordenado)
[Parada limpia (Graceful Shutdown)]:
  1. Detiene asignador de enjambre y sesiones P2P
  2. Sincroniza bloques dispersos y archivos .met
  3. Ejecuta checkpoint de SQLite
          │
          ▼  (Paso 5: Reemplazo atómico)
  - Windows: mule-daemon.exe -> mule-daemon.exe.old; mule-daemon.new -> mule-daemon.exe
  - Linux: rename atómico sobre el inodo
          │
          ▼  (Paso 6: Relanzamiento)
  - Windows/Linux autónomo: std::process::Command::spawn del nuevo binario y salida limpia
  - Linux systemd: salida limpia con código de salida esperado para reinicio de unidad
```

---

## 3. Especificación técnica del manifiesto (`update_manifest.json`)

El servidor de versiones expone un archivo JSON estático firmado:

```jsonc
{
  "version": "0.3.1",
  "minSupportedVersion": "0.2.0",
  "releaseDate": "2026-11-20T12:00:00Z",
  "channel": "stable",
  "changelogUrl": "https://github.com/caraveliko/PulseCore/releases/tag/v0.3.1",
  "summary": "Corrección de concurrencia en Kad y mejoras en el reproductor web.",
  "targets": {
    "x86_64-pc-windows-msvc": {
      "url": "https://github.com/caraveliko/PulseCore/releases/download/v0.3.1/mule-daemon-windows-x64.zip",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "size": 15728640,
      "signature": "3b6a27bc...",
      "binaryRelativePath": "mule-daemon.exe"
    },
    "x86_64-unknown-linux-gnu": {
      "url": "https://github.com/caraveliko/PulseCore/releases/download/v0.3.1/mule-daemon-linux-x86_64.tar.gz",
      "sha256": "f4c8996fb92427ae41e4649b934ca495991b7852b855e3b0c44298fc1c149a",
      "size": 14680064,
      "signature": "8a1c9e...",
      "binaryRelativePath": "mule-daemon"
    },
    "aarch64-unknown-linux-gnu": {
      "url": "https://github.com/caraveliko/PulseCore/releases/download/v0.3.1/mule-daemon-linux-aarch64.tar.gz",
      "sha256": "9afbf4c8996fb92427ae41e4649b934ca495991b7852b855e3b0c44298fc1c",
      "size": 13631488,
      "signature": "12df4a...",
      "binaryRelativePath": "mule-daemon"
    }
  }
}
```

### Reglas de validación criptográfica
1. **Clave pública maestra**: La clave pública de verificación Ed25519 de eMuleRust se compila dentro del binario como constante inmutable (`RELEASE_PUBLIC_KEY_ED25519`).
2. **Cálculo de firma**: La firma Ed25519 cubre el hash SHA-256 binario del archivo empaquetado para evitar maleabilidad del transporte.
3. **Límites de tamaño**: El cliente aborta inmediatamente la conexión si la cabecera `Content-Length` o el cuerpo recibido excede el límite máximo presupuestado (`MAX_UPDATE_PAYLOAD_BYTES`, p. ej. 64 MiB).

---

## 4. Contrato JSON-RPC (`system.*`) y eventos WebSocket

### Métodos RPC
- **`system.checkUpdate`**:
  - Parámetros: `{ "force": bool }` (opcional, por defecto `false`).
  - Resultado:
    ```jsonc
    {
      "status": "update_available", // "idle" | "checking" | "update_available" | "up_to_date" | "downloading" | "ready_to_install" | "error"
      "currentVersion": "0.3.0-beta.2",
      "latestVersion": "0.3.1",
      "releaseDate": "2026-11-20T12:00:00Z",
      "changelogUrl": "https://...",
      "summary": "...",
      "downloadSize": 15728640
    }
    ```
- **`system.getUpdateStatus`**: Devuelve la instantánea de estado del servicio actual.
- **`system.applyUpdate`**:
  - Parámetros: `{ "autoRestart": bool }` (si es `true`, tras verificar y preparar el reemplazo, inicia el apagado y relanzamiento).
  - Resultado: `{ "accepted": true, "stage": "downloading" }`.

### Eventos WebSocket emitidos
- `event.updateStatusChanged`: Notifica cambios de fase (`checking`, `downloading`, `ready_to_install`, `error`).
- `event.updateProgress`: `{ "downloadedBytes": 5242880, "totalBytes": 15728640, "percentage": 33.3 }`.

---

## 5. Configuración y persistencia (`DaemonConfig`)

Se incorporan en `crates/mule-daemon/src/config.rs`:

| Parámetro | Tipo | Valor predeterminado | Descripción |
| :--- | :--- | :--- | :--- |
| `update_check_enabled` | `bool` | `true` | Habilita la comprobación periódica en segundo plano |
| `update_channel` | `String` | `"stable"` | Canal de distribución (`"stable"`, `"beta"`) |
| `update_check_interval_secs`| `u64` | `86400` (24 h) | Intervalo entre comprobaciones automáticas |
| `update_manifest_url` | `String` | URL oficial | Endpoint HTTPS del manifiesto |
| `update_auto_download` | `bool` | `false` | Si descarga automáticamente en segundo plano o espera aprobación |

---

## 6. Procedimiento de reemplazo en caliente según sistema operativo

### Windows (x64)
1. El archivo nuevo se descomprime en `[directorio_ejecutable]\mule-daemon.update`.
2. Se renombra el ejecutable actual:
   `std::fs::rename("mule-daemon.exe", "mule-daemon.exe.old")`.
3. Se mueve el binario validado:
   `std::fs::rename("mule-daemon.update", "mule-daemon.exe")`.
4. Si `autoRestart` está activo, se invoca el nuevo proceso desacoplado (`std::process::Command`) y el proceso anterior finaliza con código de salida `0`.
5. Al siguiente arranque, `mule-daemon` detecta si existe un archivo `.old` y lo elimina silenciosamente.

### Linux (x86_64 / aarch64)
1. Se descomprime en `mule-daemon.update`.
2. Se aplican permisos de ejecución `0o755`.
3. Reemplazo atómico con `std::fs::rename("mule-daemon.update", "mule-daemon")` (en Linux, el inodo antiguo sigue activo para el proceso en memoria mientras el directorio apunta al nuevo).
4. Si el proceso corre bajo `systemd` (detectado vía variable de entorno o `systemd_notify`), finaliza limpiamente para que `systemd` lo reinicie de inmediato (`Restart=always` / `Restart=on-success`).
5. Si corre de forma independiente, ejecuta `execve` para sustituir el espacio de direcciones por el nuevo binario.

---

## 7. Plan de pruebas sin conexión (Offline QA)

Cumpliendo con la regla de pruebas 100 % sin tráfico externo:
- Creación de un servidor HTTP local en loopback (`127.0.0.1:0`) con `wiremock` o `axum` en los tests de integración.
- Generación de un par de claves Ed25519 efímeras de prueba para firmar un binario mock de prueba.
- Verificación de rechazo ante:
  - Hash SHA-256 discrepante.
  - Firma Ed25519 inválida o generada con otra clave.
  - Payload que exceda el tamaño máximo de seguridad.
  - URL no permitida por la política anti-SSRF.
- Simulación completa del ciclo de actualización y sustitución en un directorio temporal aislado.
