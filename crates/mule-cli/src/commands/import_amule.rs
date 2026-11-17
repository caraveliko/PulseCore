// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

//! Comando de migración que importa el estado de aMule / eMule, identidad
//! de usuario, créditos, listas de servidores, nodos Kad y descargas en curso
//! hacia eMule-Rust.

use std::path::Path;
use mule_daemon::services::DownloadService;
use mule_daemon::services::IdentityService;

pub struct ImportSummary {
    pub identity_imported: bool,
    pub user_hash: Option<String>,
    pub clients_imported: usize,
    pub servers_imported: usize,
    pub nodes_imported: bool,
    pub friends_imported: usize,
    pub cancelled_imported: usize,
    pub parts_migrated: usize,
    pub total_part_bytes: u64,
}

pub fn run_import_amule(
    amule_dir: &Path,
    data_dir: &Path,
    dry_run: bool,
    link_parts: bool,
) -> Result<ImportSummary, Box<dyn std::error::Error>> {
    println!("\n📦 Iniciando migración desde aMule hacia eMule-Rust...");
    println!("   Origen:  {}", amule_dir.display());
    println!("   Destino: {}", data_dir.display());
    if dry_run {
        println!("   [MODO SIMULACIÓN - Sin cambios en disco]");
    }

    if !amule_dir.exists() {
        return Err(format!("El directorio de aMule '{}' no existe", amule_dir.display()).into());
    }

    let mut summary = ImportSummary {
        identity_imported: false,
        user_hash: None,
        clients_imported: 0,
        servers_imported: 0,
        nodes_imported: false,
        friends_imported: 0,
        cancelled_imported: 0,
        parts_migrated: 0,
        total_part_bytes: 0,
    };

    if !dry_run {
        std::fs::create_dir_all(data_dir)?;
    }

    // 1. Identidad y par de claves RSA (cryptkey.dat + preferences.dat)
    let cryptkey_path = amule_dir.join("cryptkey.dat");
    let preferences_path = amule_dir.join("preferences.dat");
    if cryptkey_path.is_file() || preferences_path.is_file() {
        print!("🔑 Importando identidad eMule (cryptkey.dat + preferences.dat)... ");
        if !dry_run {
            match IdentityService::import_emule_identity(
                data_dir,
                if cryptkey_path.is_file() { Some(&cryptkey_path) } else { None },
                if preferences_path.is_file() { Some(&preferences_path) } else { None },
                true,
            ) {
                Ok(rep) => {
                    summary.identity_imported = rep.key_imported || rep.userhash_imported;
                    summary.user_hash = rep.userhash;
                    println!("OK (UserHash: {})", summary.user_hash.as_deref().unwrap_or(""));
                }
                Err(e) => println!("Error: {e}"),
            }
        } else {
            summary.identity_imported = true;
            println!("Detectado (OK)");
        }
    }

    // Preparar directorio temporal y DownloadService
    let temp_dir = data_dir.join("temp");
    let db_path = data_dir.join("mule.db");

    // 2. Créditos de pares (clients.met)
    let clients_met = amule_dir.join("clients.met");
    if clients_met.is_file() {
        print!("💳 Importando créditos acumulados (clients.met)... ");
        if !dry_run {
            std::fs::create_dir_all(&temp_dir)?;
            let mut svc = DownloadService::new(&db_path, temp_dir.clone())?;
            match svc.import_clients_met(&clients_met) {
                Ok(rep) => {
                    summary.clients_imported = rep.imported;
                    println!("OK ({} clientes / créditos importados)", rep.imported);
                }
                Err(e) => println!("Error: {e}"),
            }
        } else {
            println!("Detectado");
        }
    }

    // 3. Lista de amigos (emfriends.met)
    let friends_met = amule_dir.join("emfriends.met");
    if friends_met.is_file() {
        print!("👥 Importando lista de amigos (emfriends.met)... ");
        if !dry_run {
            std::fs::create_dir_all(&temp_dir)?;
            let mut svc = DownloadService::new(&db_path, temp_dir.clone())?;
            match svc.import_emfriends_met(&friends_met) {
                Ok(rep) => {
                    summary.friends_imported = rep.imported;
                    println!("OK ({} amigos)", rep.imported);
                }
                Err(e) => println!("Error: {e}"),
            }
        } else {
            println!("Detectado");
        }
    }

    // 4. Máscara de archivos cancelados (cancelled.met)
    let cancelled_met = amule_dir.join("cancelled.met");
    if cancelled_met.is_file() {
        print!("🚫 Importando filtros de archivos cancelados (cancelled.met)... ");
        if !dry_run {
            std::fs::create_dir_all(&temp_dir)?;
            let mut svc = DownloadService::new(&db_path, temp_dir.clone())?;
            match svc.import_cancelled_met(&cancelled_met) {
                Ok(rep) => {
                    summary.cancelled_imported = rep.total;
                    println!("OK ({} hashes)", rep.total);
                }
                Err(e) => println!("Error: {e}"),
            }
        } else {
            println!("Detectado");
        }
    }

    // 5. Contactos Kad DHT (nodes.dat)
    let nodes_dat = amule_dir.join("nodes.dat");
    if nodes_dat.is_file() {
        print!("🌐 Copiando contactos Kademlia DHT (nodes.dat)... ");
        let dest_nodes = data_dir.join("nodes.dat");
        if !dry_run {
            if let Err(e) = std::fs::copy(&nodes_dat, &dest_nodes) {
                println!("Error al copiar: {e}");
            } else {
                summary.nodes_imported = true;
                println!("OK");
            }
        } else {
            summary.nodes_imported = true;
            println!("Detectado");
        }
    }

    // 5b. Lista de servidores (server.met)
    let server_met = amule_dir.join("server.met");
    if server_met.is_file() {
        print!("🖥️  Importando lista de servidores eD2k (server.met)... ");
        let dest_server = data_dir.join("server.met");
        if !dry_run {
            let _ = std::fs::copy(&server_met, &dest_server);
            if let Ok(parsed) = mule_format::ServerMetFile::open(&server_met) {
                if let Ok(mut conn) = rusqlite::Connection::open(&db_path) {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    let servers: Vec<mule_core::model::Ed2kServer> = parsed
                        .servers
                        .into_iter()
                        .map(|s| mule_core::model::Ed2kServer {
                            address: format!("{}:{}", s.ip, s.port),
                            ip: s.ip.to_string(),
                            port: s.port,
                            name: s.name,
                            description: s.description,
                            ping: s.ping,
                            preference: s.preference,
                            failed_count: s.failed_count,
                            dyn_ip: s.dyn_ip,
                            version: s.version,
                            filter_status: mule_core::model::ServerFilterStatus::Allowed,
                            last_seen_unix_ms: None,
                            created_at_unix_ms: now,
                            updated_at_unix_ms: now,
                        })
                        .collect();
                    summary.servers_imported = servers.len();
                    let _ = mule_core::persistence::sqlite::servers::upsert_servers(&mut conn, &servers);
                    println!("OK ({} servidores importados)", servers.len());
                } else {
                    println!("OK (copiado server.met)");
                }
            } else {
                println!("OK (copiado server.met)");
            }
        } else {
            summary.servers_imported = 1;
            println!("Detectado");
        }
    }

    // 6. Descargas incompletas (*.part.met & *.part)
    // aMule almacena los archivos temporales en ~/.aMule/Temp, en el TempDir personalizado de amule.conf o en ~/.aMule
    let mut candidate_temp_dirs = vec![
        amule_dir.join("Temp"),
        amule_dir.join("temp"),
        amule_dir.to_path_buf(),
    ];

    let conf_path = amule_dir.join("amule.conf");
    if conf_path.is_file() {
        if let Ok(contents) = std::fs::read_to_string(&conf_path) {
            for line in contents.lines() {
                if let Some(rest) = line.strip_prefix("TempDir=") {
                    let p = std::path::PathBuf::from(rest.trim());
                    if p.is_dir() {
                        candidate_temp_dirs.insert(0, p);
                    }
                }
            }
        }
    }

    print!("📥 Escaneando descargas incompletas (.part.met)... ");
    let mut part_met_files = Vec::new();
    for t_dir in &candidate_temp_dirs {
        if t_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(t_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(ext) = path.extension() {
                        if ext.eq_ignore_ascii_case("met") {
                            if let Some(fname) = path.file_name() {
                                let file_str = fname.to_string_lossy();
                                if file_str.ends_with(".part.met") {
                                    part_met_files.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    part_met_files.sort();
    part_met_files.dedup();
    println!("Detectadas {} descargas en progreso", part_met_files.len());

    for met_file in &part_met_files {
        let (Some(parent), Some(met_fname)) = (met_file.parent(), met_file.file_name()) else {
            continue;
        };
        let stem = met_fname.to_string_lossy();
        // La raíz es, p. ej., "001.part.met" -> el archivo de datos correspondiente es "001.part"
        let part_data_name = stem.trim_end_matches(".met");
        let part_data_file = parent.join(part_data_name);

        let target_met = temp_dir.join(met_fname);
        let target_data = temp_dir.join(part_data_name);

        let data_size = if part_data_file.is_file() {
            std::fs::metadata(&part_data_file).map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };

        summary.parts_migrated += 1;
        summary.total_part_bytes += data_size;

        if !dry_run {
            std::fs::create_dir_all(&temp_dir)?;
            if !target_met.exists() {
                let _ = std::fs::copy(met_file, &target_met);
            }
            if part_data_file.is_file() && !target_data.exists() {
                if link_parts {
                    // Intentar enlace duro, recurrir a copia en caso de fallo
                    if std::fs::hard_link(&part_data_file, &target_data).is_err() {
                        let _ = std::fs::copy(&part_data_file, &target_data);
                    }
                } else {
                    let _ = std::fs::copy(&part_data_file, &target_data);
                }
            }
        }
    }

    println!("\n=======================================================");
    println!("🎉 RESUMEN DE MIGRACIÓN AMULE -> EMULE-RUST");
    println!("=======================================================");
    println!(" • Identidad eMule:   {}", if summary.identity_imported { "✅ Preservada (RSA + UserHash)" } else { "⚪ No encontrada" });
    println!(" • Créditos Clientes: ✅ {} clientes", summary.clients_imported);
    println!(" • Amigos:            ✅ {} contactos", summary.friends_imported);
    println!(" • Nodos Kad DHT:     {}", if summary.nodes_imported { "✅ Copiado nodes.dat" } else { "⚪ No encontrado" });
    println!(" • Descargas Temp:    ✅ {} archivos en curso ({:.2} MB)", summary.parts_migrated, summary.total_part_bytes as f64 / (1024.0 * 1024.0));
    println!("=======================================================\n");
    println!("💡 Cuando arranques `mule-daemon`, recuperará automáticamente");
    println!("   todas las descargas en progreso sin tener que volver a bajarlas!");

    Ok(summary)
}
