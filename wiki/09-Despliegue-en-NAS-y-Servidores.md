# 🖥️ 09. Despliegue en NAS y Servidores Domésticos (*Headless*)

**Pulse Core** está especialmente optimizado para ejecutarse en segundo plano (*headless*, sin pantalla ni interfaz gráfica de escritorio) en servidores domésticos, NAS comerciales o autoconstruidos, y placas monoplaca como **Raspberry Pi 4 y 5**.

---

## 1. Ventajas de la Arquitectura Headless

A diferencia de clientes pesados basados en Electron o interfaces gráficas de C++, `mule-daemon`:
- Consume apenas **~45 MB de memoria RAM en reposo**.
- No requiere X11, Wayland, Node.js ni paquetes gráficos instalados en el servidor.
- Expone toda su funcionalidad a través de un único puerto HTTP/WebSocket (`4660`), lo que facilita su acceso remoto desde cualquier móvil, tablet o PC de la casa.

---

## 2. Configuración con `systemd` en Linux

Para que el servicio arranque automáticamente al encender el servidor y se reinicie solo si ocurre algún imprevisto:

### 1. Ubicar el binario
```bash
sudo cp target/release/mule-daemon /usr/local/bin/
```

### 2. Crear el servicio de sistema
Crea el archivo `/etc/systemd/system/pulsecore.service`:

```ini
[Unit]
Description=Pulse Core P2P Daemon
After=network.target local-fs.target

[Service]
Type=simple
User=miusuario
Group=miusuario
ExecStart=/usr/local/bin/mule-daemon
Restart=on-failure
RestartSec=5s
LimitNOFILE=65535
Environment=RUST_LOG=mule_daemon=info,mule_protocol=info

[Install]
WantedBy=multi-user.target
```
*(Sustituye `miusuario` por el nombre de tu usuario habitual en el servidor).*

### 3. Habilitar y arrancar
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now pulsecore
sudo systemctl status pulsecore
```

---

## 3. Despliegue en Raspberry Pi (`aarch64`)

La Raspberry Pi 4 o 5 es una de las mejores plataformas para ejecutar Pulse Core como servidor de descargas de bajísimo consumo eléctrico (menos de 5 vatios):

### Consejos de almacenamiento en Raspberry Pi
- **No descargues en la tarjeta microSD**: Las continuas escrituras de bloques P2P pueden degradar la tarjeta de memoria a medio plazo.
- **Conecta un disco duro o SSD externo por USB 3.0**:
  - Formatea el disco en `ext4` o `btrfs`.
  - Configura el montaje automático en `/etc/fstab` (ej. en `/media/almacenamiento`).
  - En la pestaña **Ajustes** de Pulse Core, configura la carpeta temporal y la de descargas dentro de ese disco externo.

---

## 4. Consejos para TrueNAS, Unraid y Synology

Si ejecutas Pulse Core en sistemas de almacenamiento en red:

1. **Permisos de Archivo**: Asegúrate de que el usuario con el que corre el daemon tenga permisos de lectura y escritura (`rwx`) sobre la carpeta compartida donde Plex o Jellyfin leen las películas.
2. **Enlaces Duros en ZFS / Btrfs**: Tanto ZFS (TrueNAS) como Btrfs y XFS (Unraid) admiten enlaces duros (*hard links*) nativamente de forma ultra rápida. Recuerda que la carpeta temporal y la de biblioteca deben pertenecer al mismo conjunto de datos (*dataset* / sistema de archivos) para que el enlace duro funcione sin duplicar espacio.
3. **Acceso Remoto Seguro**: Si quieres acceder a la WebUI de Pulse Core fuera de tu casa, **no abras el puerto 4660 directamente a Internet sin contraseña**. En su lugar, utiliza una VPN doméstica como **WireGuard** o **Tailscale** para acceder de forma 100 % cifrada y segura.
