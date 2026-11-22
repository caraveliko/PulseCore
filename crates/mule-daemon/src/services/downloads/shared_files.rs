// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

use super::*;

impl DownloadService {
    /// Almacena en caché la raíz AICH calculada de un archivo compartido.
    pub fn set_shared_aich_root(
        &mut self,
        file_hash: &str,
        aich_root: &[u8; 20],
    ) -> Result<bool, DaemonError> {
        self.db
            .set_shared_aich_root(file_hash, aich_root)
            .map_err(db_err("setting share AICH root"))
    }

    pub fn get_completed_ranges_by_hash(&self, hex_hash: &str) -> Vec<(u64, u64)> {
        let clean_hex = hex_hash.trim().to_lowercase();
        for active in self.active.values() {
            if active.download.ed2k_hash.to_hex().to_lowercase() == clean_hex {
                return active
                    .ranges
                    .ranges()
                    .iter()
                    .map(|r| (r.start, r.end))
                    .collect();
            }
        }
        Vec::new()
    }

    pub fn list_shared_files(&self) -> Result<Vec<mule_core::model::SharedFile>, DaemonError> {
        peers_servers::list_shared_files(&self.db)
    }

    pub fn add_shared_file(
        &mut self,
        file: &mule_core::model::SharedFile,
    ) -> Result<(), DaemonError> {
        peers_servers::add_shared_file(&mut self.db, file)
    }

    /// `UAPPref` de eMule (`GetNewAutoUp`): ¿inician los nuevos archivos compartidos
    /// con prioridad de subida «auto»? (predeterminado `true`).
    pub fn auto_up_priority_enabled(&self) -> bool {
        peers_servers::auto_up_priority_enabled(&self.db)
    }

    /// Añade `delta` bytes al total subido persistido de un archivo compartido.
    pub fn add_share_uploaded_bytes(
        &mut self,
        file_hash: &str,
        delta: u64,
    ) -> Result<(), DaemonError> {
        peers_servers::add_share_uploaded_bytes(&mut self.db, file_hash, delta)
    }

    /// Totales subidos persistidos por hash de archivo (claves en minúsculas).
    pub fn list_share_uploaded_bytes(
        &self,
    ) -> Result<std::collections::HashMap<String, u64>, DaemonError> {
        peers_servers::list_share_uploaded_bytes(&self.db)
    }

    /// Retira un archivo de los recursos compartidos. Con `delete_file`, el archivo en
    /// disco se elimina adicionalmente: para una descarga completada aún
    /// activa se delega en `remove_download` (entrada en BD + archivo final +
    /// recurso compartido); de lo contrario, el archivo señalado por la entrada se elimina
    /// y luego la entrada `shared_files` — y la eventual descarga completada en BD —
    /// se retiran.
    pub fn remove_shared_file(
        &mut self,
        file_hash: &str,
        delete_file: bool,
    ) -> Result<bool, DaemonError> {
        if !delete_file {
            return peers_servers::remove_shared_file(&mut self.db, file_hash);
        }
        let sf = self
            .get_shared_file(file_hash)?
            .ok_or_else(|| DaemonError::NotFound(format!("Shared file {file_hash} not found")))?;
        if sf.is_partial {
            // Recurso compartido transitorio de una descarga en curso: el archivo a
            // eliminar es el `.part`, lo cual es función de
            // `downloads.remove` — nunca eliminarlo aquí.
            return Err(DaemonError::Business(
                "Cannot delete a partial share — remove the download instead".to_string(),
            ));
        }
        if let Some(id) = self
            .active
            .values()
            .find(|a| {
                a.download
                    .ed2k_hash
                    .to_hex()
                    .eq_ignore_ascii_case(file_hash)
            })
            .map(|a| a.download.id)
        {
            self.remove_download(&id, true)?;
            return Ok(true);
        }
        let path = PathBuf::from(&sf.file_path);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let removed = peers_servers::remove_shared_file(&mut self.db, file_hash)?;
        // Descarga completada restante en BD (fuera de `active`): su target_path
        // acaba de ser eliminada, la entrada queda muerta — la retiramos.
        let stale_db_id = self
            .db
            .get_all_downloads()
            .unwrap_or_default()
            .into_iter()
            .find(|d| {
                d.status == DownloadStatus::Completed
                    && d.ed2k_hash.to_hex().eq_ignore_ascii_case(file_hash)
            })
            .map(|d| d.id);
        if let Some(id) = stale_db_id {
            let _ = self.db.delete_download(&id);
            return Ok(true);
        }
        Ok(removed)
    }

    /// Establece la prioridad de subida de un archivo compartido ("verylow".."veryhigh").
    pub fn set_shared_up_priority(
        &mut self,
        file_hash: &str,
        up_priority: &str,
    ) -> Result<bool, DaemonError> {
        self.db
            .set_shared_up_priority(file_hash, up_priority)
            .map_err(Into::into)
    }

    pub fn get_shared_file(
        &self,
        file_hash: &str,
    ) -> Result<Option<mule_core::model::SharedFile>, DaemonError> {
        if let Some(sf) = peers_servers::get_shared_file(&self.db, file_hash)? {
            if sf.file_path.is_empty() || Path::new(&sf.file_path).is_file() {
                return Ok(Some(sf));
            }
            // Entrada persistida cuyo archivo ya no existe: no
            // servible — dejamos decidir a las fuentes sintetizadas.
        }
        for act in self.active.values() {
            if !act
                .download
                .ed2k_hash
                .to_hex()
                .eq_ignore_ascii_case(file_hash)
            {
                continue;
            }
            if act.download.status == DownloadStatus::Completed
                && !act.download.target_path.is_file()
            {
                continue;
            }
            let mut sf = Self::shared_entry_for_active(act);
            sf.aich_root = act.download.aich_root.map(|a| a.0);
            return Ok(Some(sf));
        }
        // Las descargas completadas se persisten en la BD incluso cuando no están cargadas
        // en `active` (ej.: tras un reinicio del daemon) — permanecen compartidas
        // al igual que eMule mantiene los archivos finalizados en la lista compartida.
        if let Some(sf) = self.shared_entry_for_completed_db(file_hash) {
            return Ok(Some(sf));
        }
        Ok(None)
    }

    /// Hashes actualmente presentes en la lista de recursos compartidos — `sharedfiles`
    /// de eMule: entradas activas cuyo archivo existe + descargas
    /// completadas persistidas o cargadas cuyo archivo aún existe.
    /// Los `.part` en curso **no** figuran aquí (`FindBestClientInQueue` de eMule
    /// solo consulta `sharedfiles->GetFileByID` — los clientes en espera de un
    /// archivo parcial se purgan igual que en eMule). Utilizado por
    /// `UploadManager` para purgar de la cola las solicitudes huérfanas.
    pub fn list_shared_hashes(&self) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        if let Ok(shares) = peers_servers::list_shared_files(&self.db) {
            for sf in shares {
                if sf.enabled && (sf.file_path.is_empty() || Path::new(&sf.file_path).is_file()) {
                    set.insert(sf.file_hash.to_lowercase());
                }
            }
        }
        for act in self.active.values() {
            if act.download.status == DownloadStatus::Completed
                && act.download.target_path.is_file()
            {
                set.insert(act.download.ed2k_hash.to_hex().to_lowercase());
            }
        }
        if let Ok(dls) = self.db.get_all_downloads() {
            for d in dls {
                if d.status == DownloadStatus::Completed && d.target_path.is_file() {
                    set.insert(d.ed2k_hash.to_hex().to_lowercase());
                }
            }
        }
        set
    }

    /// Resuelve una descarga completada desde la BD en una entrada compartible —
    /// únicamente cuando el archivo de destino existe realmente en disco.
    fn shared_entry_for_completed_db(
        &self,
        file_hash: &str,
    ) -> Option<mule_core::model::SharedFile> {
        let dl = self.db.get_all_downloads().ok()?.into_iter().find(|d| {
            d.status == mule_core::DownloadStatus::Completed
                && d.ed2k_hash.to_hex().eq_ignore_ascii_case(file_hash)
        })?;
        if !dl.target_path.is_file() {
            return None;
        }
        let mut sf = mule_core::model::SharedFile::new(
            dl.ed2k_hash.to_hex(),
            &dl.display_name,
            dl.total_size,
            dl.target_path.to_string_lossy().as_ref(),
        );
        sf.aich_root = dl.aich_root.map(|a| a.0);
        sf.category_id = dl.category_id.clone();
        Some(sf)
    }

    /// Construye la vista de archivo compartido de una descarga activa (semántica
    /// `CPartFile` de eMule): una descarga completada se resuelve a su ruta de destino; una
    /// descarga en progreso se resuelve a su archivo `.part` como recurso compartido parcial
    /// exponiendo únicamente sus partes eD2K completamente recibidas. Los archivos de partes
    /// siempre son archivos compartidos en eMule (`CDownloadQueue::AddPartFilesToShare`), incluso
    /// antes de que se complete cualquier parte — la máscara vacía `completed_parts` simplemente
    /// no anuncia nada servible todavía.
    fn shared_entry_for_active(act: &ActiveDownload) -> mule_core::model::SharedFile {
        if act.download.status == mule_core::DownloadStatus::Completed {
            let mut sf = mule_core::model::SharedFile::new(
                act.download.ed2k_hash.to_hex(),
                &act.download.display_name,
                act.download.total_size,
                act.download.target_path.to_string_lossy().as_ref(),
            );
            sf.category_id = act.download.category_id.clone();
            return sf;
        }
        let part_size = ED2K_PART_SIZE as u64;
        let part_count = act
            .download
            .total_size
            .div_ceil(part_size)
            .min(u16::MAX as u64) as u16;
        let completed: Vec<u16> = (0..part_count)
            .filter(|&i| {
                let start = i as u64 * part_size;
                let end = ((i as u64 + 1) * part_size).min(act.download.total_size);
                end > start && act.ranges.contains(start..end)
            })
            .collect();
        let mut sf = mule_core::model::SharedFile::new(
            act.download.ed2k_hash.to_hex(),
            &act.download.display_name,
            act.download.total_size,
            act.download.temp_path.to_string_lossy().as_ref(),
        );
        sf.is_partial = true;
        sf.completed_parts = completed;
        sf.category_id = act.download.category_id.clone();
        sf
    }

    /// Cada archivo actualmente servible a los pares: archivos compartidos persistidos más
    /// descargas activas (completadas → archivo completo, en curso → partes
    /// completadas únicamente), al igual que `CSharedFileList` de eMule incluyendo `CPartFile`s.
    pub fn list_all_shared_files(&self) -> Result<Vec<mule_core::model::SharedFile>, DaemonError> {
        let mut list = self.list_shared_files()?;
        // Los recursos compartidos persistidos cuyo archivo desapareció del disco ya
        // no son servibles — filtrados en la lectura para que ni la interfaz,
        // ni la oferta del servidor, ni la publicación Kad anuncien un
        // archivo muerto. Las entradas sin ruta (recurso compartido solo por hash)
        // se conservan.
        list.retain(|f| f.file_path.is_empty() || Path::new(&f.file_path).is_file());
        let persisted: std::collections::HashSet<String> =
            list.iter().map(|f| f.file_hash.to_lowercase()).collect();
        for act in self.active.values() {
            if persisted.contains(&act.download.ed2k_hash.to_hex().to_lowercase()) {
                continue;
            }
            // Misma salvaguarda que `shared_entry_for_completed_db`: destino
            // eliminado físicamente → nada servible a sintetizar.
            if act.download.status == DownloadStatus::Completed
                && !act.download.target_path.is_file()
            {
                continue;
            }
            let mut sf = Self::shared_entry_for_active(act);
            sf.aich_root = act.download.aich_root.map(|a| a.0);
            list.push(sf);
        }
        // Las descargas completadas solo viven en la BD tras un reinicio — fusionarlas
        // también para que la lista de compartidos sobreviva a reinicios (eMule vuelve a hashear
        // los archivos completados en `CSharedFileList` al iniciar).
        for d in self.db.get_all_downloads()? {
            if d.status != mule_core::DownloadStatus::Completed {
                continue;
            }
            if persisted.contains(&d.ed2k_hash.to_hex().to_lowercase()) {
                continue;
            }
            if let Some(sf) = self.shared_entry_for_completed_db(&d.ed2k_hash.to_hex()) {
                list.push(sf);
            }
        }
        Ok(list)
    }

    /// `shares.setCategory` — asocia (o retira) una categoría a un
    /// archivo compartido identificado por su hash eD2K. Actualiza las dos
    /// fuentes posibles: la fila `shared_files` persistida y/o la
    /// descarga (activa o completada en BD) cuyo recurso compartido se sintetiza
    /// — `list_all_shared_files` refleja la categoría desde una u
    /// otra.
    pub fn set_shared_file_category(
        &mut self,
        file_hash: &str,
        category_id: Option<String>,
    ) -> Result<(), DaemonError> {
        let cat = category_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(ref cid) = cat {
            if self.get_category(cid)?.is_none() {
                return Err(DaemonError::Business(format!("Category '{cid}' not found")));
            }
        }
        let mut touched = false;
        // Descarga activa conocida por este hash.
        let active_id = self
            .active
            .values()
            .find(|a| {
                a.download
                    .ed2k_hash
                    .to_hex()
                    .eq_ignore_ascii_case(file_hash)
            })
            .map(|a| a.download.id);
        if let Some(id) = active_id {
            self.db
                .set_download_category(&id, cat.as_deref())
                .map_err(db_err("setting category"))?;
            if let Some(a) = self.active.get_mut(&id) {
                a.download.category_id = cat.clone();
            }
            touched = true;
        } else {
            // Descarga completada presente solo en BD.
            let db_id = self
                .db
                .get_all_downloads()?
                .into_iter()
                .find(|d| d.ed2k_hash.to_hex().eq_ignore_ascii_case(file_hash))
                .map(|d| d.id);
            if let Some(id) = db_id {
                self.db
                    .set_download_category(&id, cat.as_deref())
                    .map_err(db_err("setting category"))?;
                touched = true;
            }
        }
        // Recurso compartido persistido autónomo (archivo añadido fuera de descarga).
        if self
            .db
            .set_shared_file_category(file_hash, cat.as_deref())
            .map_err(db_err("setting share category"))?
        {
            touched = true;
        }
        if !touched {
            return Err(DaemonError::Business(format!(
                "Shared file '{file_hash}' not found"
            )));
        }
        Ok(())
    }

}
