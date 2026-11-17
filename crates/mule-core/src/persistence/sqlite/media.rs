// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach" & Antigravity

use rusqlite::{params, Connection};

use crate::error::CoreError;
use crate::model::{CalendarEpisode, EpisodeItem, ExternalIndexer, MediaItem};

pub fn insert_media_item(conn: &Connection, item: &MediaItem) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO media_library (
            id, tmdb_id, imdb_id, tvdb_id, media_type, title, original_title, year,
            overview, poster_path, backdrop_path, status, monitored,
            quality_profile, root_folder, file_path, download_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
        ON CONFLICT(id) DO UPDATE SET
            title=excluded.title,
            original_title=excluded.original_title,
            year=excluded.year,
            overview=excluded.overview,
            poster_path=excluded.poster_path,
            backdrop_path=excluded.backdrop_path,
            status=excluded.status,
            monitored=excluded.monitored,
            quality_profile=excluded.quality_profile,
            root_folder=excluded.root_folder,
            file_path=COALESCE(excluded.file_path, media_library.file_path),
            download_id=excluded.download_id,
            updated_at=excluded.updated_at",
        params![
            item.id,
            item.tmdb_id,
            item.imdb_id,
            item.tvdb_id,
            item.media_type,
            item.title,
            item.original_title,
            item.year,
            item.overview,
            item.poster_path,
            item.backdrop_path,
            item.status,
            item.monitored as i32,
            item.quality_profile,
            item.root_folder,
            item.file_path,
            item.download_id,
            item.created_at,
            item.updated_at,
        ],
    )?;
    Ok(())
}

pub fn update_media_status(
    conn: &Connection,
    id: &str,
    status: &str,
    download_id: Option<&str>,
    now_ms: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE media_library SET status = ?1, download_id = ?2, updated_at = ?3 WHERE id = ?4",
        params![status, download_id, now_ms, id],
    )?;
    Ok(())
}

pub fn update_media_completed(
    conn: &Connection,
    id: &str,
    file_path: &str,
    now_ms: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE media_library SET status = 'completed', file_path = ?1, updated_at = ?2 WHERE id = ?3",
        params![file_path, now_ms, id],
    )?;
    Ok(())
}

pub fn find_media_by_download_id(
    conn: &Connection,
    download_id: &str,
) -> Result<Option<MediaItem>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, tmdb_id, imdb_id, tvdb_id, media_type, title, original_title, year,
                overview, poster_path, backdrop_path, status, monitored,
                quality_profile, root_folder, file_path, download_id, created_at, updated_at
         FROM media_library WHERE download_id = ?1",
    )?;
    let mut rows = stmt.query(params![download_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_media(row)?))
    } else {
        Ok(None)
    }
}

pub fn find_episode_by_download_id(
    conn: &Connection,
    download_id: &str,
) -> Result<Option<EpisodeItem>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, media_id, season_number, episode_number, title, air_date,
                status, monitored, file_path, download_id, created_at, updated_at
         FROM media_episodes WHERE download_id = ?1",
    )?;
    let mut rows = stmt.query(params![download_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(EpisodeItem {
            id: row.get(0)?,
            media_id: row.get(1)?,
            season_number: row.get(2)?,
            episode_number: row.get(3)?,
            title: row.get(4)?,
            air_date: row.get(5)?,
            status: row.get(6)?,
            monitored: row.get::<_, i32>(7)? != 0,
            file_path: row.get(8)?,
            download_id: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn delete_media_item(conn: &Connection, id: &str) -> Result<bool, CoreError> {
    let affected = conn.execute("DELETE FROM media_library WHERE id = ?1", params![id])?;
    let _ = conn.execute("DELETE FROM media_episodes WHERE media_id = ?1", params![id]);
    Ok(affected > 0)
}

pub fn get_media_item(conn: &Connection, id: &str) -> Result<Option<MediaItem>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, tmdb_id, imdb_id, tvdb_id, media_type, title, original_title, year,
                overview, poster_path, backdrop_path, status, monitored,
                quality_profile, root_folder, file_path, download_id, created_at, updated_at
         FROM media_library WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_media(row)?))
    } else {
        Ok(None)
    }
}

pub fn get_media_by_tmdb_id(
    conn: &Connection,
    tmdb_id: u64,
) -> Result<Option<MediaItem>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, tmdb_id, imdb_id, tvdb_id, media_type, title, original_title, year,
                overview, poster_path, backdrop_path, status, monitored,
                quality_profile, root_folder, file_path, download_id, created_at, updated_at
         FROM media_library WHERE tmdb_id = ?1",
    )?;
    let mut rows = stmt.query(params![tmdb_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_media(row)?))
    } else {
        Ok(None)
    }
}

pub fn get_media_by_tvdb_id(
    conn: &Connection,
    tvdb_id: u64,
) -> Result<Option<MediaItem>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, tmdb_id, imdb_id, tvdb_id, media_type, title, original_title, year,
                overview, poster_path, backdrop_path, status, monitored,
                quality_profile, root_folder, file_path, download_id, created_at, updated_at
         FROM media_library WHERE tvdb_id = ?1",
    )?;
    let mut rows = stmt.query(params![tvdb_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_media(row)?))
    } else {
        Ok(None)
    }
}

pub fn list_media_items(
    conn: &Connection,
    filter_type: Option<&str>,
    filter_status: Option<&str>,
) -> Result<Vec<MediaItem>, CoreError> {
    let mut sql = "SELECT id, tmdb_id, imdb_id, tvdb_id, media_type, title, original_title, year,
                          overview, poster_path, backdrop_path, status, monitored,
                          quality_profile, root_folder, file_path, download_id, created_at, updated_at
                   FROM media_library WHERE 1=1".to_string();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(t) = filter_type {
        sql.push_str(" AND media_type = ?");
        params_vec.push(Box::new(t.to_string()));
    }
    if let Some(s) = filter_status {
        sql.push_str(" AND status = ?");
        params_vec.push(Box::new(s.to_string()));
    }
    sql.push_str(" ORDER BY updated_at DESC");

    let mut stmt = conn.prepare(&sql)?;
    let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(&params_slice[..], row_to_media)?;
    let mut items = Vec::new();
    for r in rows {
        items.push(r?);
    }
    Ok(items)
}

pub fn insert_episode(conn: &Connection, ep: &EpisodeItem) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO media_episodes (
            id, media_id, season_number, episode_number, title, air_date,
            status, monitored, file_path, download_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(media_id, season_number, episode_number) DO UPDATE SET
            title=excluded.title,
            air_date=excluded.air_date,
            status=excluded.status,
            monitored=excluded.monitored,
            file_path=excluded.file_path,
            download_id=excluded.download_id,
            updated_at=excluded.updated_at",
        params![
            ep.id,
            ep.media_id,
            ep.season_number,
            ep.episode_number,
            ep.title,
            ep.air_date,
            ep.status,
            ep.monitored as i32,
            ep.file_path,
            ep.download_id,
            ep.created_at,
            ep.updated_at,
        ],
    )?;
    Ok(())
}

/// Inserción o actualización por lotes (batch) de episodios en una única transacción SQLite.
/// Evita la multiplicación de escrituras WAL y llamadas fsync al añadir o escanear una serie.
pub fn insert_episodes_batch(
    conn: &mut Connection,
    episodes: &[EpisodeItem],
) -> Result<usize, CoreError> {
    if episodes.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut count = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO media_episodes (
                id, media_id, season_number, episode_number, title, air_date,
                status, monitored, file_path, download_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(media_id, season_number, episode_number) DO UPDATE SET
                title=excluded.title,
                air_date=excluded.air_date,
                status=excluded.status,
                monitored=excluded.monitored,
                file_path=excluded.file_path,
                download_id=excluded.download_id,
                updated_at=excluded.updated_at",
        )?;

        for ep in episodes {
            stmt.execute(params![
                ep.id,
                ep.media_id,
                ep.season_number,
                ep.episode_number,
                ep.title,
                ep.air_date,
                ep.status,
                ep.monitored as i32,
                ep.file_path,
                ep.download_id,
                ep.created_at,
                ep.updated_at,
            ])?;
            count += 1;
        }
    }
    tx.commit()?;
    Ok(count)
}

pub fn get_episodes_for_media(
    conn: &Connection,
    media_id: &str,
) -> Result<Vec<EpisodeItem>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, media_id, season_number, episode_number, title, air_date,
                status, monitored, file_path, download_id, created_at, updated_at
         FROM media_episodes WHERE media_id = ?1
         ORDER BY season_number ASC, episode_number ASC",
    )?;
    let rows = stmt.query_map(params![media_id], |row| {
        Ok(EpisodeItem {
            id: row.get(0)?,
            media_id: row.get(1)?,
            season_number: row.get(2)?,
            episode_number: row.get(3)?,
            title: row.get(4)?,
            air_date: row.get(5)?,
            status: row.get(6)?,
            monitored: row.get::<_, i32>(7)? != 0,
            file_path: row.get(8)?,
            download_id: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    })?;
    let mut episodes = Vec::new();
    for r in rows {
        episodes.push(r?);
    }
    Ok(episodes)
}

pub fn get_calendar_episodes(
    conn: &Connection,
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<Vec<CalendarEpisode>, CoreError> {
    let mut sql = String::from(
        "SELECT e.id, e.media_id, e.season_number, e.episode_number, e.title, e.air_date,
                e.status, e.monitored, e.file_path, e.download_id, e.created_at, e.updated_at,
                m.title, m.poster_path, m.year
         FROM media_episodes e
         JOIN media_library m ON e.media_id = m.id
         WHERE e.air_date IS NOT NULL AND e.air_date != ''",
    );

    if start_date.is_some() {
        sql.push_str(" AND e.air_date >= :start_date");
    }
    if end_date.is_some() {
        sql.push_str(" AND e.air_date <= :end_date");
    }
    sql.push_str(" ORDER BY e.air_date ASC, e.season_number ASC, e.episode_number ASC");

    let mut stmt = conn.prepare(&sql)?;

    let s_bound = start_date.unwrap_or("");
    let e_bound = end_date.unwrap_or("");
    let mut params_vec: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
    if start_date.is_some() {
        params_vec.push((":start_date", &s_bound));
    }
    if end_date.is_some() {
        params_vec.push((":end_date", &e_bound));
    }

    let rows = stmt.query_map(&params_vec[..], |row| {
        let ep = EpisodeItem {
            id: row.get(0)?,
            media_id: row.get(1)?,
            season_number: row.get(2)?,
            episode_number: row.get(3)?,
            title: row.get(4)?,
            air_date: row.get(5)?,
            status: row.get(6)?,
            monitored: row.get::<_, i32>(7)? != 0,
            file_path: row.get(8)?,
            download_id: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        };
        let series_title: String = row.get(12)?;
        let series_poster_path: Option<String> = row.get(13)?;
        let series_year: Option<u32> = row.get(14)?;

        Ok(CalendarEpisode {
            episode: ep,
            series_title,
            series_poster_path,
            series_year,
        })
    })?;

    let mut result = Vec::new();
    for r in rows {
        result.push(r?);
    }
    Ok(result)
}

pub fn insert_external_indexer(
    conn: &Connection,
    idx: &ExternalIndexer,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO external_indexers (id, name, base_url, api_key, enabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
            name=excluded.name,
            base_url=excluded.base_url,
            api_key=excluded.api_key,
            enabled=excluded.enabled",
        params![
            idx.id,
            idx.name,
            idx.base_url,
            idx.api_key,
            idx.enabled as i32,
            idx.created_at,
        ],
    )?;
    Ok(())
}

pub fn list_external_indexers(conn: &Connection) -> Result<Vec<ExternalIndexer>, CoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, base_url, api_key, enabled, created_at FROM external_indexers ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ExternalIndexer {
            id: row.get(0)?,
            name: row.get(1)?,
            base_url: row.get(2)?,
            api_key: row.get(3)?,
            enabled: row.get::<_, i32>(4)? != 0,
            created_at: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn delete_external_indexer(conn: &Connection, id: &str) -> Result<bool, CoreError> {
    let n = conn.execute("DELETE FROM external_indexers WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

pub fn update_episode_status(
    conn: &Connection,
    id: &str,
    status: &str,
    file_path: Option<&str>,
    download_id: Option<&str>,
    now_ms: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE media_episodes SET status = ?1, file_path = COALESCE(?2, file_path), download_id = COALESCE(?3, download_id), updated_at = ?4 WHERE id = ?5",
        params![status, file_path, download_id, now_ms, id],
    )?;
    Ok(())
}

fn row_to_media(row: &rusqlite::Row) -> Result<MediaItem, rusqlite::Error> {
    Ok(MediaItem {
        id: row.get(0)?,
        tmdb_id: row.get(1)?,
        imdb_id: row.get(2)?,
        tvdb_id: row.get(3)?,
        media_type: row.get(4)?,
        title: row.get(5)?,
        original_title: row.get(6)?,
        year: row.get(7)?,
        overview: row.get(8)?,
        poster_path: row.get(9)?,
        backdrop_path: row.get(10)?,
        status: row.get(11)?,
        monitored: row.get::<_, i32>(12)? != 0,
        quality_profile: row.get(13)?,
        root_folder: row.get(14)?,
        file_path: row.get(15)?,
        download_id: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}
