// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Laurent Geynet "Loulach"

//! Persistencia SQLite del historial de notificaciones para el usuario.

use rusqlite::{params, Connection};

use crate::error::CoreError;
use crate::model::{NotificationFilter, NotificationRecord};

/// Límite predeterminado de notificaciones devueltas en una consulta sin límite explícito.
pub const DEFAULT_NOTIFICATIONS_LIMIT: usize = 50;
/// Límite máximo de notificaciones que puede devolver una sola consulta.
pub const MAX_NOTIFICATIONS_LIMIT: usize = 500;

/// Inserta una nueva notificación y la devuelve con su identificador autoincremental asignado.
pub fn insert_notification(
    conn: &mut Connection,
    record: &NotificationRecord,
) -> Result<NotificationRecord, CoreError> {
    conn.execute(
        "INSERT INTO notifications (
            kind, severity, category, title, body, metadata_json, is_read, created_at_unix_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            record.kind,
            record.severity,
            record.category,
            record.title,
            record.body,
            record.metadata_json,
            i64::from(record.is_read),
            record.created_at_unix_ms,
        ],
    )?;

    let mut inserted = record.clone();
    inserted.id = conn.last_insert_rowid();
    Ok(inserted)
}

/// Lista notificaciones ordenadas de más reciente a más antigua según el filtro indicado.
pub fn list_notifications(
    conn: &Connection,
    filter: &NotificationFilter,
) -> Result<Vec<NotificationRecord>, CoreError> {
    let mut sql = String::from(
        "SELECT id, kind, severity, category, title, body, metadata_json, is_read, created_at_unix_ms
         FROM notifications WHERE 1=1",
    );
    let mut bind_values: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(ref cat) = filter.category {
        let trimmed = cat.trim();
        if !trimmed.is_empty() && trimmed != "all" {
            sql.push_str(" AND category = ?");
            bind_values.push(rusqlite::types::Value::Text(trimmed.to_string()));
        }
    }

    if filter.unread_only {
        sql.push_str(" AND is_read = 0");
    }

    let limit = filter
        .limit
        .unwrap_or(DEFAULT_NOTIFICATIONS_LIMIT)
        .clamp(1, MAX_NOTIFICATIONS_LIMIT);

    sql.push_str(" ORDER BY created_at_unix_ms DESC, id DESC LIMIT ?");
    bind_values.push(rusqlite::types::Value::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind_values.iter()), |row| {
        let is_read_int: i64 = row.get(7)?;
        Ok(NotificationRecord {
            id: row.get(0)?,
            kind: row.get(1)?,
            severity: row.get(2)?,
            category: row.get(3)?,
            title: row.get(4)?,
            body: row.get(5)?,
            metadata_json: row.get(6)?,
            is_read: is_read_int != 0,
            created_at_unix_ms: row.get(8)?,
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Cuenta cuántas notificaciones permanecen sin leer.
pub fn count_unread_notifications(conn: &Connection) -> Result<usize, CoreError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE is_read = 0",
        [],
        |r| r.get(0),
    )?;
    Ok(count.max(0) as usize)
}

/// Marca una notificación concreta como leída. Devuelve `true` si existía la fila.
pub fn mark_notification_read(conn: &mut Connection, id: i64) -> Result<bool, CoreError> {
    let updated = conn.execute(
        "UPDATE notifications SET is_read = 1 WHERE id = ?1 AND is_read = 0",
        params![id],
    )?;
    if updated > 0 {
        return Ok(true);
    }
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(exists > 0)
}

/// Marca todas las notificaciones pendientes como leídas y devuelve el número de filas actualizadas.
pub fn mark_all_notifications_read(conn: &mut Connection) -> Result<usize, CoreError> {
    let updated = conn.execute("UPDATE notifications SET is_read = 1 WHERE is_read = 0", [])?;
    Ok(updated)
}

/// Elimina por completo el historial de notificaciones almacenado.
pub fn clear_notifications(conn: &mut Connection) -> Result<usize, CoreError> {
    let deleted = conn.execute("DELETE FROM notifications", [])?;
    Ok(deleted)
}

/// Conserva únicamente las `max_keep` notificaciones más recientes y elimina las anteriores.
pub fn prune_old_notifications(conn: &mut Connection, max_keep: usize) -> Result<usize, CoreError> {
    let deleted = if max_keep == 0 {
        conn.execute("DELETE FROM notifications", [])?
    } else {
        conn.execute(
            "DELETE FROM notifications
             WHERE id NOT IN (
                 SELECT id FROM notifications
                 ORDER BY created_at_unix_ms DESC, id DESC
                 LIMIT ?1
             )",
            params![max_keep as i64],
        )?
    };
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::sqlite::Database;

    fn sample_record(
        kind: &str,
        category: &str,
        title: &str,
        created_at_unix_ms: i64,
    ) -> NotificationRecord {
        NotificationRecord {
            id: 0,
            kind: kind.to_string(),
            severity: "info".to_string(),
            category: category.to_string(),
            title: title.to_string(),
            body: format!("Cuerpo de {title}"),
            metadata_json: None,
            is_read: false,
            created_at_unix_ms,
        }
    }

    #[test]
    fn test_notifications_crud_filters_and_prune() {
        let mut db = Database::open_in_memory().unwrap();

        let n1 = db
            .insert_notification(&sample_record("downloadCompleted", "downloads", "Primera", 1000))
            .unwrap();
        let n2 = db
            .insert_notification(&sample_record("mediaReady", "media", "Segunda", 2000))
            .unwrap();
        let n3 = db
            .insert_notification(&sample_record("diskLow", "system", "Tercera", 3000))
            .unwrap();

        assert!(n1.id > 0 && n2.id > n1.id && n3.id > n2.id);
        assert_eq!(db.count_unread_notifications().unwrap(), 3);

        // Orden descendente por marca de tiempo.
        let all = db.list_notifications(&NotificationFilter::default()).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, n3.id);
        assert_eq!(all[2].id, n1.id);

        // Filtro por categoría.
        let media_only = db
            .list_notifications(&NotificationFilter {
                category: Some("media".to_string()),
                unread_only: false,
                limit: None,
            })
            .unwrap();
        assert_eq!(media_only.len(), 1);
        assert_eq!(media_only[0].id, n2.id);

        // Marcar una como leída y filtrar no leídas.
        assert!(db.mark_notification_read(n2.id).unwrap());
        assert!(!db.mark_notification_read(999_999).unwrap());
        assert_eq!(db.count_unread_notifications().unwrap(), 2);

        let unread = db
            .list_notifications(&NotificationFilter {
                category: None,
                unread_only: true,
                limit: None,
            })
            .unwrap();
        assert_eq!(unread.len(), 2);
        assert!(!unread.iter().any(|n| n.id == n2.id));

        // Podar manteniendo solo las 2 más recientes (n3 y n2).
        let pruned = db.prune_old_notifications(2).unwrap();
        assert_eq!(pruned, 1);
        let remaining = db.list_notifications(&NotificationFilter::default()).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].id, n3.id);
        assert_eq!(remaining[1].id, n2.id);

        // Marcar todas como leídas y limpiar por completo.
        let marked = db.mark_all_notifications_read().unwrap();
        assert_eq!(marked, 1);
        assert_eq!(db.count_unread_notifications().unwrap(), 0);

        let cleared = db.clear_notifications().unwrap();
        assert_eq!(cleared, 2);
        assert!(db
            .list_notifications(&NotificationFilter::default())
            .unwrap()
            .is_empty());
    }
}
