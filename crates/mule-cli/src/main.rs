// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use mule_cli::commands::{hash, import_amule, known_met, link, part_met, update};

fn home_subdir(leaf: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(leaf)
}

#[derive(Parser)]
#[command(name = "mule-cli")]
#[command(about = "Inspection CLI for eD2K links, hashing, and eMule metadata files", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspecciona y valida un enlace ed2k://
    InspectLink {
        /// La cadena URI ed2k://
        link: String,
    },
    /// Calcula hashes deterministas eD2K y AICH para un archivo local
    Hash {
        /// Ruta al archivo
        file: PathBuf,
    },
    /// Inspecciona un archivo .part.met de eMule
    InspectPartMet {
        /// Ruta al archivo .part.met
        file: PathBuf,
    },
    /// Inspecciona un archivo known.met de eMule (índice de archivos compartidos)
    InspectKnownMet {
        /// Ruta al archivo known.met
        file: PathBuf,
    },
    /// Genera un enlace de archivo ed2k:// a partir de un archivo local
    MakeLink {
        /// Ruta al archivo local
        file: PathBuf,
        /// Muestra información detallada junto al enlace
        #[arg(long, short)]
        detailed: bool,
    },
    /// Migra el estado de aMule / eMule (créditos, identidad, nodos y descargas activas)
    ImportAmule {
        /// Ruta al directorio de configuración de aMule
        #[arg(long, default_value_os_t = home_subdir(".aMule"))]
        amule_dir: PathBuf,

        /// Directorio de datos de destino de eMule-Rust
        #[arg(long, default_value_os_t = home_subdir(".mule"))]
        data_dir: PathBuf,

        /// Modo de simulación (simula la migración sin modificar el disco)
        #[arg(long)]
        dry_run: bool,

        /// Crear enlaces duros (hardlinks) para los archivos part en lugar de copiar para ahorrar espacio en disco
        #[arg(long)]
        link_parts: bool,
    },
    /// Consulta, comprueba, sube o revierte actualizaciones en un demonio Pulse Core
    Update {
        /// URL base HTTP del demonio en ejecución
        #[arg(long, default_value = update::DEFAULT_DAEMON_URL)]
        url: String,
        /// Token Bearer de autenticación (si se omite, lee MULE_AUTH_TOKEN o auth.token)
        #[arg(long)]
        token: Option<String>,
        /// Directorio de datos del demonio donde reside `auth.token`
        #[arg(long, default_value_os_t = home_subdir(".mule"))]
        data_dir: PathBuf,
        #[command(subcommand)]
        action: update::UpdateAction,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::InspectLink { link: link_uri } => {
            link::inspect_link(&link_uri);
        }
        Commands::Hash { file } => {
            hash::hash_file(&file)?;
        }
        Commands::InspectPartMet { file } => {
            part_met::inspect_part_met(&file);
        }
        Commands::InspectKnownMet { file } => {
            known_met::inspect_known_met(&file);
        }
        Commands::MakeLink { file, detailed } => {
            hash::make_link(&file, detailed)?;
        }
        Commands::ImportAmule {
            amule_dir,
            data_dir,
            dry_run,
            link_parts,
        } => {
            import_amule::run_import_amule(&amule_dir, &data_dir, dry_run, link_parts)?;
        }
        Commands::Update {
            url,
            token,
            data_dir,
            action,
        } => {
            update::run_update(&url, token.as_deref(), &data_dir, action)?;
        }
    }

    Ok(())
}
