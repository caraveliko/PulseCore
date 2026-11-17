// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Entorno efímero aislado y neutralizado para la ejecución de referencia externa de nivel 3 (Tier 3).
//!
//! Impone:
//! - Aislamiento en directorio temporal efímero (nunca toca AppData del usuario ni rutas del proyecto).
//! - Neutralización estricta de la red externa (Kad desactivado, listas de servidores vacías y desactivadas, autoconexión desactivada).
//! - Vinculación explícita de puertos en el bucle local (loopback).

#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Tipo de binario de referencia externo — define el formato de configuración neutralizada
/// (`preferences.ini` para eMule, `amule.conf` para aMule) y el estilo de lanzamiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ReferenceKind {
    /// eMule (GUI de Windows) — configuración mediante `preferences.ini` + fijación en registro.
    Emule,
    /// aMule (demonio headless `amuled`) — configuración mediante `amule.conf` + `-c <dir>`.
    Amule,
}

/// Representa un entorno de ejecución aislado para un binario de referencia de eMule/aMule.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Tier3Environment {
    work_dir: TempDir,
    config_dir: PathBuf,
    incoming_dir: PathBuf,
    temp_dir: PathBuf,
    tcp_port: u16,
    udp_port: u16,
}

impl Tier3Environment {
    /// Crea un nuevo entorno aislado con configuración neutralizada.
    pub fn new(tcp_port: u16, udp_port: u16) -> io::Result<Self> {
        Self::new_with_kind(ReferenceKind::Emule, tcp_port, udp_port)
    }

    /// Creates a new isolated environment with neutralized configuration
    /// adapted to the reference binary kind.
    pub fn new_with_kind(
        kind: ReferenceKind,
        tcp_port: u16,
        udp_port: u16,
    ) -> io::Result<Self> {
        let work_dir = tempfile::tempdir()?;
        let work_path = work_dir.path();

        let config_dir = work_path.join("config");
        let incoming_dir = work_path.join("incoming");
        let temp_dir = work_path.join("temp");

        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&incoming_dir)?;
        fs::create_dir_all(&temp_dir)?;

        let env = Self {
            work_dir,
            config_dir,
            incoming_dir,
            temp_dir,
            tcp_port,
            udp_port,
        };

        match kind {
            ReferenceKind::Emule => env.write_emule_preferences_ini()?,
            ReferenceKind::Amule => env.write_amule_conf()?,
        }
        env.write_empty_server_met()?;

        Ok(env)
    }

    /// Ruta al directorio de trabajo raíz aislado.
    pub fn work_path(&self) -> &Path {
        self.work_dir.path()
    }

    /// Ruta al directorio de configuración.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Ruta al directorio de descargas completadas (incoming).
    pub fn incoming_dir(&self) -> &Path {
        &self.incoming_dir
    }

    /// Ruta al directorio de archivos temporales de partes (.part).
    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    /// El puerto TCP configurado para el par de referencia.
    pub fn tcp_port(&self) -> u16 {
        self.tcp_port
    }

    /// El puerto UDP configurado para el par de referencia.
    pub fn udp_port(&self) -> u16 {
        self.udp_port
    }

    /// Crea un archivo fixture determinista en el directorio incoming con bytes repetitivos 0..=255.
    pub fn create_deterministic_fixture(
        &self,
        filename: &str,
        length: usize,
    ) -> io::Result<PathBuf> {
        use std::io::Write;
        let file_path = self.incoming_dir.join(filename);
        let mut file = fs::File::create(&file_path)?;

        let chunk_size = 64 * 1024;
        let mut buf = vec![0u8; chunk_size];
        let mut written = 0;
        while written < length {
            let to_write = std::cmp::min(chunk_size, length - written);
            for (i, item) in buf.iter_mut().enumerate().take(to_write) {
                *item = ((written + i) % 256) as u8;
            }
            file.write_all(&buf[..to_write])?;
            written += to_write;
        }
        file.flush()?;
        Ok(file_path)
    }

    /// Escribe un archivo `preferences.ini` neutralizado para eMule.
    fn write_emule_preferences_ini(&self) -> io::Result<()> {
        let ini_content = format!(
            "[eMule]\r\n\
AppVersion=0.72a\r\n\
UsePublicUserDirectories=0\r\n\
FirstRuntime=0\r\n\
ShowSplashScreen=0\r\n\
StartMinimized=1\r\n\
Nick=RefPeer\r\n\
Port={tcp_port}\r\n\
UDPPort={udp_port}\r\n\
FilterLANIPs=0\r\n\
EnableKad=0\r\n\
NetworkED2K=1\r\n\
NetworkKademlia=0\r\n\
AutoConnect=0\r\n\
AutoUpdateServerList=0\r\n\
ServerUDPPort=0\r\n\
UpdateNotify=0\r\n\
CheckDiskspace=0\r\n\
MinFreeDiskSpace=0\r\n\
SafeServerConnect=1\r\n\
AutoConnectStaticOnly=1\r\n\
Verbose=1\r\n\
IncomingDir={incoming}\r\n\
TempDir={temp}\r\n",
            tcp_port = self.tcp_port,
            udp_port = self.udp_port,
            incoming = self.incoming_dir.display(),
            temp = self.temp_dir.display()
        );
        fs::write(self.config_dir.join("preferences.ini"), &ini_content)?;
        fs::write(self.work_dir.path().join("preferences.ini"), &ini_content)?;
        Ok(())
    }

    /// Escribe un archivo `amule.conf` neutralizado para aMule.
    ///
    /// aMule lee `amule.conf` en la raíz del directorio pasado mediante `-c`. Todas
    /// las formas de autoconexión se neutralizan; las claves desconocidas son
    /// simplemente ignoradas por wxConfig (defensa en profundidad).
    fn write_amule_conf(&self) -> io::Result<()> {
        // `amuled` se niega a funcionar sin Conexiones Externas (EC) — activamos
        // EC confinándolo estrictamente a loopback mediante `ECAddress`, con un
        // puerto efímero asignado por el SO y una contraseña de prueba (MD5 hex).
        let ec_port = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .and_then(|l| l.local_addr().map(|a| a.port()))
            .unwrap_or(47120);
        // MD5("tier3ref") — contraseña EC simulada, nunca reutilizada.
        const EC_PASSWORD_MD5: &str = "c876a623b75f060ad300fff1b90531b0";
        let conf_content = format!(
            "[eMule]\r\n\
AppVersion=3.1.0\r\n\
Nick=RefPeer\r\n\
QueueSizePref=50\r\n\
VerboseDebug=1\r\n\
Verbose=1\r\n\
Port={tcp_port}\r\n\
UDPPort={udp_port}\r\n\
UDPEnable=1\r\n\
TempDir={temp}\r\n\
IncomingDir={incoming}\r\n\
OnlineSignatureDir={incoming}\r\n\
ICH=1\r\n\
AICHTrust=0\r\n\
CheckDiskspace=0\r\n\
MinFreeDiskSpace=1\r\n\
AddNewFilesPaused=0\r\n\
PreviewPrio=0\r\n\
ManualHighID=0\r\n\
ConnectToED2K=0\r\n\
ConnectToKad=0\r\n\
Autoconnect=0\r\n\
Reconnect=0\r\n\
Serverlist=0\r\n\
NewVersionCheck=0\r\n\
UpdateNotify=0\r\n\
GeoIPEnabled=0\r\n\
GeoIPAutoUpdate=0\r\n\
UPnPEnabled=0\r\n\
FilterLanIPs=0\r\n\
IPFilterON=0\r\n\
\r\n\
[ExternalConnect]\r\n\
AcceptExternalConnections=1\r\n\
ECUseTCPPort=1\r\n\
ECAddress=127.0.0.1\r\n\
ECPort={ec_port}\r\n\
ECPassword={EC_PASSWORD_MD5}\r\n\
UPnPECEnabled=0\r\n\
\r\n\
[WebServer]\r\n\
Enabled=0\r\n\
\r\n\
[UPnP]\r\n\
UPnPEnabled=0\r\n\
UPnPTCPPort=50000\r\n",
            tcp_port = self.tcp_port,
            udp_port = self.udp_port,
            incoming = self.incoming_dir.display(),
            temp = self.temp_dir.display(),
            ec_port = ec_port,
        );
        fs::write(self.config_dir.join("amule.conf"), &conf_content)?;
        fs::write(self.work_dir.path().join("amule.conf"), &conf_content)?;

        // `shareddir.dat`: lista de texto de los directorios compartidos (una línea por
        // ruta) — requerido para que aMule indexe los fixtures de `incoming/`.
        let shareddir = format!("{}\r\n", self.incoming_dir.display());
        fs::write(self.config_dir.join("shareddir.dat"), &shareddir)?;
        fs::write(self.work_dir.path().join("shareddir.dat"), &shareddir)?;
        Ok(())
    }

    /// Escribe un archivo `server.met` vacío para prevenir intentos de conexión a servidores.
    fn write_empty_server_met(&self) -> io::Result<()> {
        let server_met_path = self.config_dir.join("server.met");
        // Cabecera server.met de eMule: byte de versión 0xE0 + 4 bytes de recuento de servidores (0)
        let empty_met: [u8; 5] = [0xE0, 0x00, 0x00, 0x00, 0x00];
        fs::write(server_met_path, empty_met)
    }
}
