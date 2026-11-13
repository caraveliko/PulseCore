// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use mule_cli::commands::{hash, import_amule, known_met, link, part_met};

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
    /// Inspect and validate an ed2k:// link
    InspectLink {
        /// The ed2k:// URI string
        link: String,
    },
    /// Compute deterministic eD2K and AICH hashes for a local file
    Hash {
        /// Path to the file
        file: PathBuf,
    },
    /// Inspect an eMule .part.met file
    InspectPartMet {
        /// Path to the .part.met file
        file: PathBuf,
    },
    /// Inspect an eMule known.met file (shared-files index)
    InspectKnownMet {
        /// Path to the known.met file
        file: PathBuf,
    },
    /// Generate an ed2k:// file link from a local file
    MakeLink {
        /// Path to the local file
        file: PathBuf,
        /// Output detailed information alongside the link
        #[arg(long, short)]
        detailed: bool,
    },
    /// Migrate aMule / eMule state (credits, identity, nodes, and active downloads)
    ImportAmule {
        /// Path to the aMule configuration directory
        #[arg(long, default_value_os_t = home_subdir(".aMule"))]
        amule_dir: PathBuf,

        /// Destination eMule-Rust data directory
        #[arg(long, default_value_os_t = home_subdir(".mule"))]
        data_dir: PathBuf,

        /// Dry run mode (simulate migration without modifying disk)
        #[arg(long)]
        dry_run: bool,

        /// Hardlink part files instead of copying to save disk space
        #[arg(long)]
        link_parts: bool,
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
    }

    Ok(())
}
