// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use rusqlite::{params, Connection};
use std::path::Path;
use uuid::Uuid;

use crate::error::CoreError;
use crate::kad::{KadContact, KadIndexedNote, KadIndexedSource, KadKeywordEntry};
use crate::model::{
    CalendarEpisode, Download, DownloadCategory, DownloadStatus, Ed2kServer, EpisodeItem, ExternalIndexer, Friend,
    HistoryEntry, HistoryFilter, HistoryStats, MediaItem, PeerCredit, PeerSource,
    PeerSourceStatus, SharedFile,
};
use crate::persistence::traits::{
    DownloadRepository, KadRepository, PeerSourceRepository, ServerRepository,
};
use crate::ranges::RangeSet;

pub mod cancelled;
pub mod categories;
pub mod credits;
pub mod downloads;
pub mod friends;
pub mod history;
pub mod kad;
pub mod media;
pub mod schema;
pub mod servers;
pub mod shares;
pub mod sources;

/// Tiempo de espera de un bloqueo SQLite antes de `SQLITE_BUSY` — bajo
/// contienda de disco (antivirus, puntos de control WAL), una escritura no debe
/// fallar inmediatamente.
const SQLITE_BUSY_TIMEOUT_MS: i64 = 5_000;
/// Tamaño de la caché de páginas SQLite (negativo = KiB) — 2 MiB (perfil de memoria mínimo para embebido/RPi).
const SQLITE_CACHE_SIZE_KIB: i64 = -2_048;
/// Umbral de punto de control automático del diario WAL en páginas (~2 MiB a 4 KiB/página)
/// — acota la relectura WAL en la carga y la magnitud de un punto de control para limitar el desgaste de Flash/SSD y la RAM del diario.
const SQLITE_WAL_AUTOCHECKPOINT_PAGES: i64 = 500;
/// Tamaño máximo de E/S proyectada en memoria (mmap) para SQLite (4 MiB) — reduce el espacio de direccionamiento virtual
/// y la huella residente (RSS) en Linux/aarch64 sin perjudicar la velocidad de las consultas locales.
const SQLITE_MMAP_SIZE_BYTES: i64 = 4_194_304;

/// Gestor de persistencia de base de datos para mule-core.
pub struct Database {
    conn: Connection,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// Abre o crea una base de datos SQLite en modo WAL.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, CoreError> {
        let conn = Connection::open(path)?;

        // Activa el modo WAL para alto rendimiento y durabilidad frente a caídas
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", SQLITE_BUSY_TIMEOUT_MS)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "cache_size", SQLITE_CACHE_SIZE_KIB)?;
        conn.pragma_update(None, "wal_autocheckpoint", SQLITE_WAL_AUTOCHECKPOINT_PAGES)?;
        conn.pragma_update(None, "mmap_size", SQLITE_MMAP_SIZE_BYTES)?;

        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Auxiliar de base de datos en memoria para pruebas.
    pub fn open_in_memory() -> Result<Self, CoreError> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), CoreError> {
        schema::init_schema(&self.conn)
    }

    // ── Ajustes ─────────────────────────────────────────────────────────────

    /// Obtiene el valor de un ajuste por clave.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            let val: String = row.get(0)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// Establece o sustituye el valor de un ajuste.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Elimina un ajuste por clave.
    pub fn remove_setting(&mut self, key: &str) -> Result<bool, CoreError> {
        let rows = self
            .conn
            .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(rows > 0)
    }

    /// Lista todos los ajustes clave-valor almacenados en SQLite.
    pub fn list_settings(&self) -> Result<Vec<(String, String)>, CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM settings ORDER BY key ASC")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Descargas ───────────────────────────────────────────────────────────

    /// Guarda un nuevo registro de descarga.
    pub fn insert_download(&mut self, dl: &Download) -> Result<(), CoreError> {
        downloads::insert_download(&mut self.conn, dl)
    }

    /// Persiste el hash raíz AICH de confianza para una descarga.
    pub fn update_aich_root(
        &mut self,
        download_id: &Uuid,
        aich_root: &[u8; 20],
    ) -> Result<(), CoreError> {
        downloads::update_aich_root(&mut self.conn, download_id, aich_root)
    }

    /// Sustituye atómicamente los rangos recibidos para una descarga en una sola transacción.
    /// Puntos de compactación/desalojo exclusivamente — la ruta crítica de recepción
    /// utiliza [`Self::append_received_range`].
    pub fn sync_received_ranges(
        &mut self,
        download_id: &Uuid,
        ranges: &RangeSet,
    ) -> Result<(), CoreError> {
        downloads::sync_received_ranges(&mut self.conn, download_id, ranges)
    }

    /// Persiste un rango recién recibido en O(1) (upsert, sin
    /// DELETE+INSERT global) — ver [`downloads::append_received_range`].
    pub fn append_received_range(
        &mut self,
        download_id: &Uuid,
        start: u64,
        end: u64,
    ) -> Result<(), CoreError> {
        downloads::append_received_range(&mut self.conn, download_id, start, end)
    }

    /// Carga el RangeSet almacenado para una descarga.
    pub fn load_received_ranges(&self, download_id: &Uuid) -> Result<RangeSet, CoreError> {
        downloads::load_received_ranges(&self.conn, download_id)
    }

    /// Registra que una parte ha sido verificada.
    pub fn mark_part_verified(
        &mut self,
        download_id: &Uuid,
        part_index: u32,
    ) -> Result<(), CoreError> {
        downloads::mark_part_verified(&mut self.conn, download_id, part_index)
    }

    /// Desmarca una parte tras detectar corrupción.
    pub fn unmark_part_verified(
        &mut self,
        download_id: &Uuid,
        part_index: u32,
    ) -> Result<(), CoreError> {
        downloads::unmark_part_verified(&mut self.conn, download_id, part_index)
    }

    /// Recupera todos los índices de partes verificadas.
    pub fn get_verified_parts(&self, download_id: &Uuid) -> Result<Vec<u32>, CoreError> {
        downloads::get_verified_parts(&self.conn, download_id)
    }

    /// Almacena el conjunto de hashes MD4 por parte para un archivo (indexado por el hash eD2K hexadecimal en minúsculas).
    pub fn store_part_hashes(
        &mut self,
        file_hash: &str,
        part_hashes: &[[u8; 16]],
    ) -> Result<(), CoreError> {
        downloads::store_part_hashes(&mut self.conn, file_hash, part_hashes)
    }

    /// Carga el conjunto de hashes MD4 por parte para un archivo, ordenado por índice de parte.
    pub fn get_part_hashes(&self, file_hash: &str) -> Result<Vec<[u8; 16]>, CoreError> {
        downloads::get_part_hashes(&self.conn, file_hash)
    }

    /// Elimina el conjunto de hashes por parte almacenado para un archivo.
    pub fn delete_part_hashes(&mut self, file_hash: &str) -> Result<(), CoreError> {
        downloads::delete_part_hashes(&mut self.conn, file_hash)
    }

    /// Actualiza el estado de una descarga.
    pub fn update_status(
        &mut self,
        download_id: &Uuid,
        status: DownloadStatus,
    ) -> Result<(), CoreError> {
        downloads::update_status(&mut self.conn, download_id, status)
    }

    /// Persiste la prioridad por descarga ("low" | "normal" | "high").
    pub fn update_priority(
        &mut self,
        download_id: &Uuid,
        priority: &str,
    ) -> Result<(), CoreError> {
        downloads::update_priority(&mut self.conn, download_id, priority)
    }

    /// Persiste el límite de fuentes por archivo (`FT_MAXSOURCES` de eMule).
    /// `None` restaura el límite predeterminado global.
    pub fn update_max_sources(
        &mut self,
        download_id: &Uuid,
        max_sources: Option<u32>,
    ) -> Result<(), CoreError> {
        downloads::update_max_sources(&mut self.conn, download_id, max_sources)
    }

    /// Recupera todas las descargas persistidas desde la base de datos.
    pub fn get_all_downloads(&self) -> Result<Vec<Download>, CoreError> {
        downloads::get_all_downloads(&self.conn)
    }

    /// Elimina un registro de descarga y sus rangos y partes verificadas asociados.
    pub fn delete_download(&mut self, download_id: &Uuid) -> Result<bool, CoreError> {
        downloads::delete_download(&mut self.conn, download_id)
    }

    /// Actualiza la ruta de finalización objetivo de una descarga.
    pub fn update_target_path(
        &mut self,
        download_id: &Uuid,
        target_path: &Path,
    ) -> Result<(), CoreError> {
        downloads::update_target_path(&mut self.conn, download_id, target_path)
    }

    /// Cambia el nombre de una descarga: actualiza `display_name` y la
    /// `target_path` derivada (la finalización escribe con el nuevo nombre).
    pub fn update_download_name_and_target(
        &mut self,
        download_id: &Uuid,
        display_name: &str,
        target_path: &Path,
    ) -> Result<(), CoreError> {
        downloads::update_download_name_and_target(
            &mut self.conn,
            download_id,
            display_name,
            target_path,
        )
    }

    // ── Historial de descargas ────────────────────────────────────────────────────

    /// Inserta un nuevo registro de historial y devuelve el ID de fila generado.
    pub fn insert_history_entry(&mut self, entry: &HistoryEntry) -> Result<i64, CoreError> {
        history::insert_history_entry(&mut self.conn, entry)
    }

    /// Consulta entradas de historial que coinciden con el filtro indicado.
    pub fn query_history(&self, filter: &HistoryFilter) -> Result<Vec<HistoryEntry>, CoreError> {
        history::query_history(&self.conn, filter)
    }

    /// Agrega estadísticas históricas a través de ventanas de tiempo (sesión, 24 h, 7 d, histórico total).
    pub fn get_history_stats(&self, session_start_unix_ms: i64) -> Result<HistoryStats, CoreError> {
        history::get_history_stats(&self.conn, session_start_unix_ms)
    }

    /// Elimina todos los registros de historial.
    pub fn clear_history(&mut self) -> Result<usize, CoreError> {
        history::clear_history(&mut self.conn)
    }

    /// Purga los registros de historial con una antigüedad superior a los días de retención especificados.
    pub fn apply_retention_policy(&mut self, retention_days: u32) -> Result<usize, CoreError> {
        history::apply_retention_policy(&mut self.conn, retention_days)
    }

    // ── Categorías ──────────────────────────────────────────────────────────

    /// Recupera todas las categorías ordenadas por sort_order ascendente y después created_at ascendente.
    pub fn get_categories(&self) -> Result<Vec<DownloadCategory>, CoreError> {
        categories::get_categories(&self.conn)
    }

    /// Recupera una categoría por su ID.
    pub fn get_category(&self, id: &str) -> Result<Option<DownloadCategory>, CoreError> {
        categories::get_category(&self.conn, id)
    }

    /// Inserta una nueva categoría.
    pub fn insert_category(&mut self, cat: &DownloadCategory) -> Result<(), CoreError> {
        categories::insert_category(&mut self.conn, cat)
    }

    /// Actualiza una categoría existente.
    pub fn update_category(&mut self, cat: &DownloadCategory) -> Result<bool, CoreError> {
        categories::update_category(&mut self.conn, cat)
    }

    /// Elimina una categoría y reasigna atómicamente a NULL todas las descargas afectadas.
    pub fn delete_category(&mut self, id: &str) -> Result<usize, CoreError> {
        categories::delete_category(&mut self.conn, id)
    }

    /// Reordena las categorías basándose en la lista ordenada de ID de categoría proporcionada.
    pub fn reorder_categories(&mut self, category_ids: &[String]) -> Result<(), CoreError> {
        categories::reorder_categories(&mut self.conn, category_ids)
    }

    /// Asigna o desasigna la categoría vinculada a una descarga.
    pub fn set_download_category(
        &mut self,
        download_id: &Uuid,
        category_id: Option<&str>,
    ) -> Result<bool, CoreError> {
        categories::set_download_category(&mut self.conn, download_id, category_id)
    }

    // ── Servidores ─────────────────────────────────────────────────────────────

    /// Inserta o actualiza un lote de servidores eD2K.
    pub fn upsert_servers(&mut self, servers: &[Ed2kServer]) -> Result<usize, CoreError> {
        servers::upsert_servers(&mut self.conn, servers)
    }

    /// Lista todos los servidores eD2K persistidos ordenados por preference DESC, name ASC.
    pub fn list_servers(&self) -> Result<Vec<Ed2kServer>, CoreError> {
        servers::list_servers(&self.conn)
    }

    /// Actualiza el estado habilitado/deshabilitado de un servidor.
    pub fn set_server_enabled(&mut self, address: &str, enabled: bool) -> Result<bool, CoreError> {
        servers::set_server_enabled(&mut self.conn, address, enabled)
    }

    /// Elimina un servidor de SQLite por dirección ("ip:port").
    pub fn delete_server(&mut self, address: &str) -> Result<bool, CoreError> {
        servers::delete_server(&mut self.conn, address)
    }

    /// Elimina todos los servidores de SQLite.
    pub fn clear_servers(&mut self) -> Result<usize, CoreError> {
        servers::clear_servers(&mut self.conn)
    }

    // ── Registro de fuentes de pares ───────────────────────────────────────────────

    /// Inserta o actualiza un lote de fuentes de pares para un hash de archivo dado.
    pub fn upsert_peer_sources(
        &mut self,
        file_hash: &str,
        sources: &[PeerSource],
    ) -> Result<usize, CoreError> {
        sources::upsert_peer_sources(&mut self.conn, file_hash, sources)
    }

    /// Lista todas las fuentes de pares persistidas para un hash de archivo dado.
    pub fn list_peer_sources(&self, file_hash: &str) -> Result<Vec<PeerSource>, CoreError> {
        sources::list_peer_sources(&self.conn, file_hash)
    }

    /// Actualiza el estado, fail_count, last_error y enfriamiento opcional de una fuente de par.
    pub fn update_peer_source_status(
        &mut self,
        file_hash: &str,
        address: &str,
        status: PeerSourceStatus,
        error: Option<&str>,
        cooldown_until_unix_ms: Option<i64>,
    ) -> Result<bool, CoreError> {
        sources::update_peer_source_status(
            &mut self.conn,
            file_hash,
            address,
            status,
            error,
            cooldown_until_unix_ms,
        )
    }

    /// Actualiza el nombre de software conocido y/o la máscara de bits de disponibilidad de una fuente de par.
    pub fn update_peer_source_details(
        &mut self,
        file_hash: &str,
        address: &str,
        software: Option<&str>,
        availability: Option<&[u8]>,
    ) -> Result<bool, CoreError> {
        sources::update_peer_source_details(
            &mut self.conn,
            file_hash,
            address,
            software,
            availability,
        )
    }

    /// Actualiza las coordenadas de servidor eD2K de una fuente de par.
    pub fn update_peer_source_server(
        &mut self,
        file_hash: &str,
        address: &str,
        server_ip: u32,
        server_port: u16,
    ) -> Result<bool, CoreError> {
        sources::update_peer_source_server(
            &mut self.conn,
            file_hash,
            address,
            server_ip,
            server_port,
        )
    }

    /// Restablece el enfriamiento y el recuento de fallos de una fuente de par.
    pub fn reset_peer_source_cooldown(
        &mut self,
        file_hash: &str,
        address: &str,
    ) -> Result<bool, CoreError> {
        sources::reset_peer_source_cooldown(&mut self.conn, file_hash, address)
    }

    /// Elimina una fuente de par específica de SQLite.
    pub fn delete_peer_source(
        &mut self,
        file_hash: &str,
        address: &str,
    ) -> Result<bool, CoreError> {
        sources::delete_peer_source(&mut self.conn, file_hash, address)
    }

    /// Elimina todas las fuentes de pares para un hash de archivo.
    pub fn clear_peer_sources(&mut self, file_hash: &str) -> Result<usize, CoreError> {
        sources::clear_peer_sources(&mut self.conn, file_hash)
    }

    // ── Archivos compartidos ────────────────────────────────────────────────────────

    /// Inserta o actualiza un archivo compartido en SQLite.
    pub fn upsert_shared_file(&mut self, file: &SharedFile) -> Result<(), CoreError> {
        shares::upsert_shared_file(&mut self.conn, file)
    }

    /// Inserta o actualiza un lote de archivos compartidos en una sola transacción SQLite.
    pub fn upsert_shared_files_batch(&mut self, files: &[SharedFile]) -> Result<usize, CoreError> {
        shares::upsert_shared_files_batch(&mut self.conn, files)
    }

    /// Lista todos los archivos compartidos en SQLite.
    pub fn list_shared_files(&self) -> Result<Vec<SharedFile>, CoreError> {
        shares::list_shared_files(&self.conn)
    }

    /// Recupera un archivo compartido específico por hash.
    pub fn get_shared_file(&self, file_hash: &str) -> Result<Option<SharedFile>, CoreError> {
        shares::get_shared_file(&self.conn, file_hash)
    }

    /// Elimina una entrada de archivo compartido de SQLite.
    pub fn delete_shared_file(&mut self, file_hash: &str) -> Result<bool, CoreError> {
        shares::delete_shared_file(&mut self.conn, file_hash)
    }

    /// Alterna el estado habilitado de un archivo compartido.
    pub fn set_shared_file_enabled(
        &mut self,
        file_hash: &str,
        enabled: bool,
    ) -> Result<bool, CoreError> {
        shares::set_shared_file_enabled(&mut self.conn, file_hash, enabled)
    }

    /// Añade `delta` bytes al total subido persistido de un archivo compartido.
    pub fn add_share_uploaded_bytes(
        &mut self,
        file_hash: &str,
        delta: u64,
    ) -> Result<(), CoreError> {
        shares::add_share_uploaded_bytes(&mut self.conn, file_hash, delta)
    }

    /// Totales subidos persistidos por hash de archivo (claves en minúsculas).
    pub fn list_share_uploaded_bytes(
        &self,
    ) -> Result<std::collections::HashMap<String, u64>, CoreError> {
        shares::list_share_uploaded_bytes(&self.conn)
    }

    /// Asocia (o desvincula, `None`) una categoría a un recurso compartido persistido.
    pub fn set_shared_file_category(
        &mut self,
        file_hash: &str,
        category_id: Option<&str>,
    ) -> Result<bool, CoreError> {
        shares::set_shared_file_category(&mut self.conn, file_hash, category_id)
    }

    /// Establece la prioridad de subida (`up_priority`) de un archivo compartido.
    pub fn set_shared_up_priority(
        &mut self,
        file_hash: &str,
        up_priority: &str,
    ) -> Result<bool, CoreError> {
        shares::set_shared_up_priority(&mut self.conn, file_hash, up_priority)
    }

    /// Almacena en caché el hash raíz AICH calculado de un archivo compartido.
    pub fn set_shared_aich_root(
        &mut self,
        file_hash: &str,
        aich_root: &[u8; 20],
    ) -> Result<bool, CoreError> {
        shares::set_shared_aich_root(&mut self.conn, file_hash, aich_root)
    }

    // ── Créditos de pares ────────────────────────────────────────────────────────

    /// Inserta o actualiza un registro de créditos de par en SQLite.
    pub fn upsert_peer_credit(&mut self, credit: &PeerCredit) -> Result<(), CoreError> {
        credits::upsert_peer_credit(&mut self.conn, credit)
    }

    /// Recupera un registro de créditos de par por hash de clave pública.
    pub fn get_peer_credit(&self, pubkey_hash: &str) -> Result<Option<PeerCredit>, CoreError> {
        credits::get_peer_credit(&self.conn, pubkey_hash)
    }

    /// Incrementa el contador de bytes subidos para un registro de créditos de par.
    pub fn record_credit_upload(
        &mut self,
        pubkey_hash: &str,
        pubkey_hex: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        credits::record_credit_upload(&mut self.conn, pubkey_hash, pubkey_hex, bytes)
    }

    /// Incrementa el contador de bytes descargados para un registro de créditos de par.
    pub fn record_credit_download(
        &mut self,
        pubkey_hash: &str,
        pubkey_hex: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        credits::record_credit_download(&mut self.conn, pubkey_hash, pubkey_hex, bytes)
    }

    /// Lista los mejores créditos de pares.
    pub fn list_peer_credits(&self, limit: usize) -> Result<Vec<PeerCredit>, CoreError> {
        credits::list_peer_credits(&self.conn, limit)
    }

    // ── Amigos ─────────────────────────────────────────────────────────────

    /// Añade o actualiza un amigo (equivalente a `friends.met` de eMule).
    pub fn add_friend(&mut self, user_hash: &str, name: &str) -> Result<(), CoreError> {
        friends::add_friend(&mut self.conn, user_hash, name)
    }

    /// Elimina un amigo por hash de usuario.
    pub fn remove_friend(&mut self, user_hash: &str) -> Result<(), CoreError> {
        friends::remove_friend(&mut self.conn, user_hash)
    }

    /// Lista todos los amigos.
    pub fn list_friends(&self) -> Result<Vec<Friend>, CoreError> {
        friends::list_friends(&self.conn)
    }

    // ── Archivos cancelados (m_mapCancelledFiles de eMule) ───────────────────────

    /// `m_dwCancelledFilesSeed` local — generado perezosamente y persistido,
    /// al igual que en eMule (`Preferences.cpp` solo lo escribe en `cancelled.met`,
    /// pero persistirlo en los ajustes mantiene reproducible cada máscara).
    pub fn get_or_create_cancelled_seed(&mut self) -> Result<u32, CoreError> {
        if let Some(raw) = self.get_setting(cancelled::CANCELLED_SEED_KEY)? {
            if let Ok(seed) = raw.parse::<u32>() {
                if seed != 0 {
                    return Ok(seed);
                }
            }
        }
        // eMule: `theApp.GetRandomUInt32() % 0xFFFFFFFE + 1`.
        let seed = (rand::random::<u32>() % 0xFFFF_FFFE) + 1;
        self.set_setting(cancelled::CANCELLED_SEED_KEY, &seed.to_string())?;
        Ok(seed)
    }

    /// Registra un hash enmascarado bajo `seed` (idempotente).
    pub fn add_cancelled_hash(&mut self, seed: u32, masked_hash_hex: &str) -> Result<(), CoreError> {
        cancelled::add_cancelled_hash(&mut self.conn, seed, masked_hash_hex)
    }

    /// `true` cuando `masked_hash_hex` existe bajo `seed`.
    pub fn is_cancelled_masked(&self, seed: u32, masked_hash_hex: &str) -> Result<bool, CoreError> {
        cancelled::is_cancelled_masked(&self.conn, seed, masked_hash_hex)
    }

    /// Todas las semillas distintas presentes en el conjunto de cancelados.
    pub fn cancelled_seeds(&self) -> Result<Vec<u32>, CoreError> {
        cancelled::distinct_seeds(&self.conn)
    }

    /// Número de hashes enmascarados almacenados.
    pub fn cancelled_count(&self) -> Result<usize, CoreError> {
        cancelled::cancelled_count(&self.conn)
    }

    // ── Contactos y enrutamiento Kad ──────────────────────────────────────────────

    /// Inserta o actualiza un registro de contacto Kad.
    pub fn upsert_kad_contact(&mut self, contact: &KadContact) -> Result<(), CoreError> {
        kad::upsert_kad_contact(&mut self.conn, contact)
    }

    /// Inserta o actualiza en lote contactos Kad en una sola transacción.
    pub fn batch_upsert_kad_contacts(
        &mut self,
        contacts: &[KadContact],
    ) -> Result<usize, CoreError> {
        kad::batch_upsert_kad_contacts(&mut self.conn, contacts)
    }

    /// Recupera todos los contactos Kad hasta un límite determinado.
    pub fn list_kad_contacts(&self, limit: usize) -> Result<Vec<KadContact>, CoreError> {
        kad::list_kad_contacts(&self.conn, limit)
    }

    /// Cuenta el total de contactos Kad almacenados en SQLite.
    pub fn count_kad_contacts(&self) -> Result<usize, CoreError> {
        kad::count_kad_contacts(&self.conn)
    }

    /// Elimina todos los contactos Kad.
    pub fn clear_kad_contacts(&mut self) -> Result<usize, CoreError> {
        kad::clear_kad_contacts(&mut self.conn)
    }

    // ── Fuentes indexadas Kad ─────────────────────────────────────────────────

    /// Inserta o sustituye un lote de fuentes indexadas Kad.
    pub fn insert_kad_indexed_sources(
        &mut self,
        sources: &[KadIndexedSource],
    ) -> Result<usize, CoreError> {
        kad::insert_kad_indexed_sources(&mut self.conn, sources)
    }

    /// Recupera fuentes Kad indexadas no expiradas para un hash de archivo dado.
    pub fn get_kad_indexed_sources(
        &self,
        file_hash: &[u8; 16],
        max_results: usize,
        now_unix_ms: u64,
    ) -> Result<Vec<KadIndexedSource>, CoreError> {
        kad::get_kad_indexed_sources(&self.conn, file_hash, max_results, now_unix_ms)
    }

    /// Carga todas las fuentes Kad indexadas activas (rehidratación del nodo de almacenamiento).
    pub fn list_all_kad_indexed_sources(
        &self,
        now_unix_ms: u64,
        limit: usize,
    ) -> Result<Vec<KadIndexedSource>, CoreError> {
        kad::list_all_kad_indexed_sources(&self.conn, now_unix_ms, limit)
    }

    /// Purga las fuentes indexadas Kad expiradas.
    pub fn purge_expired_kad_sources(&mut self, now_unix_ms: u64) -> Result<usize, CoreError> {
        kad::purge_expired_kad_sources(&mut self.conn, now_unix_ms)
    }

    /// Cuenta el total de fuentes indexadas Kad.
    pub fn count_kad_indexed_sources(&self) -> Result<usize, CoreError> {
        kad::count_kad_indexed_sources(&self.conn)
    }

    // ── Índice de palabras clave Kad ───────────────────────────────────────────────────

    /// Inserta o actualiza en lote entradas del índice de palabras clave Kad.
    pub fn insert_kad_keyword_entries(
        &mut self,
        entries: &[KadKeywordEntry],
    ) -> Result<usize, CoreError> {
        kad::insert_kad_keyword_entries(&mut self.conn, entries)
    }

    /// Recupera todas las entradas activas del índice de palabras clave Kad que coinciden con un hash de palabra clave.
    pub fn get_kad_keyword_entries(
        &self,
        keyword_hash: &str,
        now_unix_ms: u64,
    ) -> Result<Vec<KadKeywordEntry>, CoreError> {
        kad::get_kad_keyword_entries(&self.conn, keyword_hash, now_unix_ms)
    }

    /// Carga todas las entradas de palabras clave Kad activas (rehidratación del nodo de almacenamiento).
    pub fn list_all_kad_keyword_entries(
        &self,
        now_unix_ms: u64,
        limit: usize,
    ) -> Result<Vec<KadKeywordEntry>, CoreError> {
        kad::list_all_kad_keyword_entries(&self.conn, now_unix_ms, limit)
    }

    /// Purga las entradas de palabras clave Kad expiradas.
    pub fn purge_expired_kad_keywords(&mut self, now_unix_ms: u64) -> Result<usize, CoreError> {
        kad::purge_expired_kad_keywords(&mut self.conn, now_unix_ms)
    }

    /// Cuenta el total de entradas de palabras clave Kad indexadas.
    pub fn count_kad_keyword_entries(&self) -> Result<usize, CoreError> {
        kad::count_kad_keyword_entries(&self.conn)
    }

    /// Inserta o actualiza entradas de notas Kad (rol de nodo de almacenamiento).
    pub fn insert_kad_notes(&mut self, notes: &[KadIndexedNote]) -> Result<usize, CoreError> {
        kad::insert_kad_notes(&mut self.conn, notes)
    }

    /// Carga todas las entradas de notas Kad activas para rehidratar el almacén.
    pub fn list_all_kad_notes(
        &self,
        now_unix_ms: u64,
        limit: usize,
    ) -> Result<Vec<KadIndexedNote>, CoreError> {
        kad::list_all_kad_notes(&self.conn, now_unix_ms, limit)
    }

    /// Purga las entradas de notas Kad expiradas.
    pub fn purge_expired_kad_notes(&mut self, now_unix_ms: u64) -> Result<usize, CoreError> {
        kad::purge_expired_kad_notes(&mut self.conn, now_unix_ms)
    }

    /// Cuenta el total de entradas de notas Kad indexadas.
    pub fn count_kad_notes(&self) -> Result<usize, CoreError> {
        kad::count_kad_notes(&self.conn)
    }
}

impl DownloadRepository for Database {
    fn insert_download(&mut self, download: &Download) -> Result<(), CoreError> {
        downloads::insert_download(&mut self.conn, download)
    }

    fn get_download(&self, id: &Uuid) -> Result<Option<Download>, CoreError> {
        downloads::get_download(&self.conn, id)
    }

    fn get_all_downloads(&self) -> Result<Vec<Download>, CoreError> {
        downloads::get_all_downloads(&self.conn)
    }

    fn update_status(&mut self, id: &Uuid, status: DownloadStatus) -> Result<(), CoreError> {
        downloads::update_status(&mut self.conn, id, status)
    }

    fn delete_download(&mut self, id: &Uuid) -> Result<bool, CoreError> {
        downloads::delete_download(&mut self.conn, id)
    }

    fn save_received_ranges(&mut self, id: &Uuid, ranges: &RangeSet) -> Result<(), CoreError> {
        downloads::sync_received_ranges(&mut self.conn, id, ranges)
    }

    fn load_received_ranges(&self, id: &Uuid) -> Result<RangeSet, CoreError> {
        downloads::load_received_ranges(&self.conn, id)
    }
}

impl PeerSourceRepository for Database {
    fn upsert_peer_sources(
        &mut self,
        file_hash_hex: &str,
        sources: &[PeerSource],
    ) -> Result<usize, CoreError> {
        sources::upsert_peer_sources(&mut self.conn, file_hash_hex, sources)
    }

    fn list_peer_sources(&self, file_hash_hex: &str) -> Result<Vec<PeerSource>, CoreError> {
        sources::list_peer_sources(&self.conn, file_hash_hex)
    }

    fn update_peer_source_status(
        &mut self,
        file_hash_hex: &str,
        address: &str,
        status: PeerSourceStatus,
        error: Option<&str>,
        cooldown_until_unix_ms: Option<i64>,
    ) -> Result<bool, CoreError> {
        sources::update_peer_source_status(
            &mut self.conn,
            file_hash_hex,
            address,
            status,
            error,
            cooldown_until_unix_ms,
        )
    }

    fn delete_peer_source(
        &mut self,
        file_hash_hex: &str,
        address: &str,
    ) -> Result<bool, CoreError> {
        sources::delete_peer_source(&mut self.conn, file_hash_hex, address)
    }
}

impl ServerRepository for Database {
    fn list_servers(&self) -> Result<Vec<Ed2kServer>, CoreError> {
        servers::list_servers(&self.conn)
    }

    fn upsert_servers(&mut self, servers: &[Ed2kServer]) -> Result<usize, CoreError> {
        servers::upsert_servers(&mut self.conn, servers)
    }

    fn delete_server(&mut self, address: &str) -> Result<bool, CoreError> {
        servers::delete_server(&mut self.conn, address)
    }

    fn clear_servers(&mut self) -> Result<usize, CoreError> {
        servers::clear_servers(&mut self.conn)
    }
}

impl KadRepository for Database {
    fn upsert_kad_contact(&mut self, contact: &KadContact) -> Result<(), CoreError> {
        kad::upsert_kad_contact(&mut self.conn, contact)
    }

    fn list_kad_contacts(&self, limit: usize) -> Result<Vec<KadContact>, CoreError> {
        kad::list_kad_contacts(&self.conn, limit)
    }

    fn insert_kad_sources(&mut self, sources: &[KadIndexedSource]) -> Result<usize, CoreError> {
        kad::insert_kad_indexed_sources(&mut self.conn, sources)
    }

    fn get_kad_sources(
        &self,
        target_kad_id: &[u8; 16],
        limit: usize,
        now_ms: u64,
    ) -> Result<Vec<KadIndexedSource>, CoreError> {
        kad::get_kad_indexed_sources(&self.conn, target_kad_id, limit, now_ms)
    }
}

impl Database {
    pub fn insert_media_item(&self, item: &MediaItem) -> Result<(), CoreError> {
        media::insert_media_item(&self.conn, item)
    }

    pub fn update_media_status(
        &self,
        id: &str,
        status: &str,
        download_id: Option<&str>,
        now_ms: i64,
    ) -> Result<(), CoreError> {
        media::update_media_status(&self.conn, id, status, download_id, now_ms)
    }

    pub fn delete_media_item(&self, id: &str) -> Result<bool, CoreError> {
        media::delete_media_item(&self.conn, id)
    }

    pub fn get_media_item(&self, id: &str) -> Result<Option<MediaItem>, CoreError> {
        media::get_media_item(&self.conn, id)
    }

    pub fn get_media_by_tmdb_id(&self, tmdb_id: u64) -> Result<Option<MediaItem>, CoreError> {
        media::get_media_by_tmdb_id(&self.conn, tmdb_id)
    }

    pub fn get_media_by_tvdb_id(&self, tvdb_id: u64) -> Result<Option<MediaItem>, CoreError> {
        media::get_media_by_tvdb_id(&self.conn, tvdb_id)
    }

    pub fn list_media_items(
        &self,
        filter_type: Option<&str>,
        filter_status: Option<&str>,
    ) -> Result<Vec<MediaItem>, CoreError> {
        media::list_media_items(&self.conn, filter_type, filter_status)
    }

    pub fn insert_episode(&self, ep: &EpisodeItem) -> Result<(), CoreError> {
        media::insert_episode(&self.conn, ep)
    }

    pub fn insert_episodes_batch(&mut self, episodes: &[EpisodeItem]) -> Result<usize, CoreError> {
        media::insert_episodes_batch(&mut self.conn, episodes)
    }

    pub fn get_episodes_for_media(&self, media_id: &str) -> Result<Vec<EpisodeItem>, CoreError> {
        media::get_episodes_for_media(&self.conn, media_id)
    }

    pub fn get_calendar_episodes(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<Vec<CalendarEpisode>, CoreError> {
        media::get_calendar_episodes(&self.conn, start_date, end_date)
    }

    pub fn insert_external_indexer(&self, idx: &ExternalIndexer) -> Result<(), CoreError> {
        media::insert_external_indexer(&self.conn, idx)
    }

    pub fn list_external_indexers(&self) -> Result<Vec<ExternalIndexer>, CoreError> {
        media::list_external_indexers(&self.conn)
    }

    pub fn delete_external_indexer(&self, id: &str) -> Result<bool, CoreError> {
        media::delete_external_indexer(&self.conn, id)
    }

    pub fn update_episode_status(
        &self,
        id: &str,
        status: &str,
        file_path: Option<&str>,
        download_id: Option<&str>,
        now_ms: i64,
    ) -> Result<(), CoreError> {
        media::update_episode_status(&self.conn, id, status, file_path, download_id, now_ms)
    }

    pub fn update_media_completed(
        &self,
        id: &str,
        file_path: &str,
        now_ms: i64,
    ) -> Result<(), CoreError> {
        media::update_media_completed(&self.conn, id, file_path, now_ms)
    }

    pub fn find_media_by_download_id(
        &self,
        download_id: &str,
    ) -> Result<Option<MediaItem>, CoreError> {
        media::find_media_by_download_id(&self.conn, download_id)
    }

    pub fn find_episode_by_download_id(
        &self,
        download_id: &str,
    ) -> Result<Option<EpisodeItem>, CoreError> {
        media::find_episode_by_download_id(&self.conn, download_id)
    }
}
