use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::clock::Clock;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde_json::Value;

use crate::model::{
    Conversation, Direction, Issue, Media, Message, MessageStatus, Rendered, StatusEvent, Webhook,
};

const MIGRATIONS: &[&str] = &[r"
    CREATE TABLE messages (
        id TEXT PRIMARY KEY,
        app_id TEXT NOT NULL,
        contact TEXT NOT NULL,
        direction TEXT NOT NULL,
        type TEXT NOT NULL,
        status TEXT NOT NULL,
        payload TEXT NOT NULL,
        rendered TEXT NOT NULL,
        issues TEXT NOT NULL DEFAULT '[]',
        error TEXT,
        context_id TEXT,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX messages_conversation ON messages (app_id, contact, created_at);
    CREATE TABLE statuses (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        message_id TEXT NOT NULL,
        status TEXT NOT NULL,
        error TEXT,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX statuses_message ON statuses (message_id);
    CREATE TABLE contacts (
        app_id TEXT NOT NULL,
        wa_id TEXT NOT NULL,
        name TEXT,
        PRIMARY KEY (app_id, wa_id)
    );
    CREATE TABLE media (
        id TEXT PRIMARY KEY,
        app_id TEXT NOT NULL,
        message_id TEXT,
        mime_type TEXT NOT NULL,
        filename TEXT,
        sha256 TEXT NOT NULL,
        file_size INTEGER NOT NULL,
        path TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX media_message ON media (message_id);
    CREATE TABLE webhooks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        app_id TEXT NOT NULL,
        url TEXT NOT NULL,
        event TEXT NOT NULL,
        message_id TEXT,
        body TEXT NOT NULL,
        status_code INTEGER,
        response TEXT,
        error TEXT,
        duration_ms INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX webhooks_message ON webhooks (message_id);
"];

pub struct Store {
    db: Mutex<Connection>,
    media_dir: PathBuf,
    clock: Arc<dyn Clock>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Store {
    pub fn open(data_dir: &Path, clock: Arc<dyn Clock>) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir)?;
        let data_dir = std::fs::canonicalize(data_dir)?;
        let media_dir = data_dir.join("media");
        std::fs::create_dir_all(&media_dir)?;

        let db = Connection::open(data_dir.join("wapit.sqlite"))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&db)?;

        Ok(Self {
            db: Mutex::new(db),
            media_dir,
            clock,
        })
    }

    pub fn media_dir(&self) -> &Path {
        &self.media_dir
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Saves a message with its contact and, for inbound media, the media row, in one transaction.
    /// When it fails the media file already written to disk is removed, so nothing is left half recorded.
    pub fn record_message(
        &self,
        message: &Message,
        contact_name: Option<&str>,
        media: Option<&Media>,
    ) -> Result<(), StoreError> {
        let result = self.record_message_rows(message, contact_name, media);

        if result.is_err()
            && let Some(media) = media
        {
            remove_files(vec![media.path.clone()]);
        }

        result
    }

    fn record_message_rows(
        &self,
        message: &Message,
        contact_name: Option<&str>,
        media: Option<&Media>,
    ) -> Result<(), StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        upsert_contact(
            &transaction,
            &message.app_id,
            &message.contact,
            contact_name,
        )?;
        insert_message(&transaction, message)?;

        if let Some(media) = media {
            insert_media(&transaction, media, Some(&message.id))?;
        }

        transaction.commit()?;

        Ok(())
    }

    /// Saves a file the app uploaded, removing it from disk when the row cannot be written.
    pub fn record_upload(&self, media: &Media) -> Result<(), StoreError> {
        let result = insert_media(&self.db(), media, None);

        if result.is_err() {
            remove_files(vec![media.path.clone()]);
        }

        result
    }

    pub fn message(&self, id: &str) -> Result<Option<Message>, StoreError> {
        Ok(self
            .db()
            .query_row("SELECT * FROM messages WHERE id = ?1", [id], to_message)
            .optional()?)
    }

    pub fn conversation(&self, app_id: &str, contact: &str) -> Result<Vec<Message>, StoreError> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT * FROM messages WHERE app_id = ?1 AND contact = ?2 ORDER BY created_at, rowid",
        )?;
        let rows = statement.query_map([app_id, contact], to_message)?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn last_inbound_at(&self, app_id: &str, contact: &str) -> Result<Option<i64>, StoreError> {
        Ok(self.db().query_row(
            "SELECT MAX(created_at) FROM messages WHERE app_id = ?1 AND contact = ?2 AND direction = 'in'",
            [app_id, contact],
            |row| row.get(0),
        )?)
    }

    pub fn conversations(&self) -> Result<Vec<(Conversation, Option<Message>)>, StoreError> {
        let db = self.db();
        let flagged = MessageStatus::FLAGGED
            .iter()
            .map(|status| format!("'{status}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = db.prepare(&format!(
            "SELECT m.app_id, m.contact, c.name, MAX(m.created_at) AS last_at, COUNT(*) AS total,
                SUM(CASE WHEN m.issues != '[]' OR m.status IN ({flagged}) THEN 1 ELSE 0 END) AS flagged,
                (SELECT id FROM messages l WHERE l.app_id = m.app_id AND l.contact = m.contact ORDER BY l.created_at DESC, l.rowid DESC LIMIT 1) AS last_id
             FROM messages m
             LEFT JOIN contacts c ON c.app_id = m.app_id AND c.wa_id = m.contact
             GROUP BY m.app_id, m.contact
             ORDER BY last_at DESC"
        ))?;
        let conversations = statement
            .query_map([], |row| {
                Ok(Conversation {
                    app_id: row.get("app_id")?,
                    contact: row.get("contact")?,
                    name: row.get("name")?,
                    last_at: row.get("last_at")?,
                    total: row.get("total")?,
                    flagged: row.get("flagged")?,
                    last_id: row.get("last_id")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut last = db.prepare("SELECT * FROM messages WHERE id = ?1")?;

        conversations
            .into_iter()
            .map(|conversation| {
                let message = last
                    .query_row([&conversation.last_id], to_message)
                    .optional()?;
                Ok((conversation, message))
            })
            .collect()
    }

    /// Records the status event and moves the message forward. Returns `false` when the message no longer exists.
    pub fn record_status(
        &self,
        id: &str,
        status: &MessageStatus,
        error: Option<&Value>,
        at: i64,
    ) -> Result<bool, StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;

        let current: Option<String> = transaction
            .query_row("SELECT status FROM messages WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;

        let Some(current) = current else {
            return Ok(false);
        };

        if status.rank() >= MessageStatus::parse(&current).rank() {
            transaction.execute(
                "UPDATE messages SET status = ?1 WHERE id = ?2",
                params![status.as_str(), id],
            )?;
        }

        transaction.execute(
            "INSERT INTO statuses (message_id, status, error, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, status.as_str(), error.map(Value::to_string), at],
        )?;
        transaction.commit()?;

        Ok(true)
    }

    pub fn set_inbound_status(&self, id: &str, status: &MessageStatus) -> Result<(), StoreError> {
        let db = self.db();
        db.execute(
            "UPDATE messages SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )?;
        db.execute(
            "INSERT INTO statuses (message_id, status, error, created_at) VALUES (?1, ?2, NULL, ?3)",
            params![id, status.as_str(), self.clock.now_ms()],
        )?;

        Ok(())
    }

    pub fn statuses(&self, message_ids: &[String]) -> Result<Vec<StatusEvent>, StoreError> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.db();
        let sql = format!(
            "SELECT * FROM statuses WHERE message_id IN ({}) ORDER BY created_at, id",
            placeholders(message_ids.len())
        );
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(message_ids), |row| {
            Ok(StatusEvent {
                id: row.get("id")?,
                message_id: row.get("message_id")?,
                status: MessageStatus::parse(&row.get::<_, String>("status")?),
                error: json_column(row, "error")?,
                created_at: row.get("created_at")?,
            })
        })?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn contact_name(&self, app_id: &str, wa_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT name FROM contacts WHERE app_id = ?1 AND wa_id = ?2",
                [app_id, wa_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn media(&self, id: &str) -> Result<Option<Media>, StoreError> {
        Ok(self
            .db()
            .query_row("SELECT * FROM media WHERE id = ?1", [id], |row| {
                Ok(Media {
                    id: row.get("id")?,
                    app_id: row.get("app_id")?,
                    mime_type: row.get("mime_type")?,
                    filename: row.get("filename")?,
                    sha256: row.get("sha256")?,
                    file_size: row.get("file_size")?,
                    path: row.get("path")?,
                    created_at: row.get("created_at")?,
                })
            })
            .optional()?)
    }

    /// Removes an uploaded media row and returns its file path so the caller can delete the file.
    pub fn delete_media(&self, id: &str) -> Result<Option<String>, StoreError> {
        let db = self.db();
        let path: Option<String> = db
            .query_row("SELECT path FROM media WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;

        if path.is_some() {
            db.execute("DELETE FROM media WHERE id = ?1", [id])?;
        }

        Ok(path)
    }

    pub fn insert_webhook(&self, webhook: &Webhook) -> Result<i64, StoreError> {
        let db = self.db();
        db.execute(
            "INSERT INTO webhooks (app_id, url, event, message_id, body, status_code, response, error, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                webhook.app_id,
                webhook.url,
                webhook.event,
                webhook.message_id,
                webhook.body,
                webhook.status_code,
                webhook.response,
                webhook.error,
                webhook.duration_ms,
                webhook.created_at
            ],
        )?;

        Ok(db.last_insert_rowid())
    }

    pub fn webhook(&self, id: i64) -> Result<Option<Webhook>, StoreError> {
        Ok(self
            .db()
            .query_row("SELECT * FROM webhooks WHERE id = ?1", [id], to_webhook)
            .optional()?)
    }

    pub fn webhooks_for_messages(
        &self,
        message_ids: &[String],
    ) -> Result<Vec<Webhook>, StoreError> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.db();
        let sql = format!(
            "SELECT * FROM webhooks WHERE message_id IN ({}) ORDER BY id",
            placeholders(message_ids.len())
        );
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(message_ids), to_webhook)?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Deletes a conversation with its statuses, webhooks and media files.
    pub fn delete_conversation(&self, app_id: &str, contact: &str) -> Result<(), StoreError> {
        self.delete_where("app_id = ?1 AND contact = ?2", &[app_id, contact])
    }

    /// Deletes everything, including uploaded media no message points to and webhooks without a message.
    pub fn delete_all(&self) -> Result<(), StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let paths: Vec<String> = {
            let mut statement = transaction.prepare("SELECT path FROM media")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };

        transaction.execute_batch("DELETE FROM statuses; DELETE FROM webhooks; DELETE FROM media; DELETE FROM messages; DELETE FROM contacts;")?;
        transaction.commit()?;
        drop(db);
        remove_files(paths);

        Ok(())
    }

    fn delete_where(&self, filter: &str, values: &[&str]) -> Result<(), StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let scope = format!("SELECT id FROM messages WHERE {filter}");

        let paths: Vec<String> = {
            let mut statement = transaction.prepare(&format!(
                "SELECT path FROM media WHERE message_id IN ({scope})"
            ))?;
            statement
                .query_map(params_from_iter(values), |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };

        for table in ["statuses", "webhooks", "media"] {
            transaction.execute(
                &format!("DELETE FROM {table} WHERE message_id IN ({scope})"),
                params_from_iter(values),
            )?;
        }

        transaction.execute(
            &format!("DELETE FROM messages WHERE {filter}"),
            params_from_iter(values),
        )?;

        if values.is_empty() {
            transaction.execute("DELETE FROM contacts", [])?;
        }

        transaction.commit()?;
        drop(db);
        remove_files(paths);

        Ok(())
    }
}

fn insert_message(db: &Connection, message: &Message) -> Result<(), StoreError> {
    db.execute(
        "INSERT INTO messages (id, app_id, contact, direction, type, status, payload, rendered, issues, error, context_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            message.id,
            message.app_id,
            message.contact,
            message.direction.as_str(),
            message.kind,
            message.status.as_str(),
            message.payload.to_string(),
            serde_json::to_string(&message.rendered).unwrap_or_default(),
            serde_json::to_string(&message.issues).unwrap_or_default(),
            message.error.as_ref().map(Value::to_string),
            message.context_id,
            message.created_at,
        ],
    )?;

    Ok(())
}

fn upsert_contact(
    db: &Connection,
    app_id: &str,
    wa_id: &str,
    name: Option<&str>,
) -> Result<(), StoreError> {
    db.execute(
        "INSERT INTO contacts (app_id, wa_id, name) VALUES (?1, ?2, ?3)
         ON CONFLICT (app_id, wa_id) DO UPDATE SET name = COALESCE(excluded.name, contacts.name)",
        params![app_id, wa_id, name],
    )?;

    Ok(())
}

fn insert_media(
    db: &Connection,
    media: &Media,
    message_id: Option<&str>,
) -> Result<(), StoreError> {
    db.execute(
        "INSERT INTO media (id, app_id, message_id, mime_type, filename, sha256, file_size, path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            media.id,
            media.app_id,
            message_id,
            media.mime_type,
            media.filename,
            media.sha256,
            media.file_size,
            media.path,
            media.created_at
        ],
    )?;

    Ok(())
}

fn remove_files(paths: Vec<String>) {
    for path in paths {
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove media file {path}: {error}");
        }
    }
}

fn migrate(db: &Connection) -> Result<(), rusqlite::Error> {
    adopt_unversioned_database(db)?;

    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (index, migration) in MIGRATIONS
        .iter()
        .enumerate()
        .skip(usize::try_from(version).unwrap_or(0))
    {
        db.execute_batch(&format!(
            "BEGIN; {migration} PRAGMA user_version = {}; COMMIT;",
            index + 1
        ))?;
    }

    Ok(())
}

/// Databases written by the first release have the schema of migration 1 without `user_version`
/// and without `media.message_id`, so they are upgraded in place instead of recreated.
fn adopt_unversioned_database(db: &Connection) -> Result<(), rusqlite::Error> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let has_messages: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'messages')",
        [],
        |row| row.get(0),
    )?;

    if version != 0 || !has_messages {
        return Ok(());
    }

    let has_message_id: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM pragma_table_info('media') WHERE name = 'message_id')",
        [],
        |row| row.get(0),
    )?;

    db.execute_batch(&format!(
        "BEGIN;
         {}
         CREATE INDEX IF NOT EXISTS media_message ON media (message_id);
         CREATE INDEX IF NOT EXISTS webhooks_message ON webhooks (message_id);
         UPDATE media SET message_id = (
             SELECT m.id FROM messages m
             WHERE m.direction = 'in' AND json_extract(m.payload, '$.' || m.type || '.id') = media.id
         ) WHERE message_id IS NULL;
         PRAGMA user_version = 1;
         COMMIT;",
        if has_message_id { "" } else { "ALTER TABLE media ADD COLUMN message_id TEXT;" }
    ))
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(",")
}

fn json_column(row: &Row<'_>, column: &str) -> rusqlite::Result<Option<Value>> {
    let raw: Option<String> = row.get(column)?;

    Ok(raw.and_then(|raw| serde_json::from_str(&raw).ok()))
}

fn to_message(row: &Row<'_>) -> rusqlite::Result<Message> {
    let direction: String = row.get("direction")?;
    let payload: String = row.get("payload")?;
    let rendered: String = row.get("rendered")?;
    let issues: String = row.get("issues")?;

    Ok(Message {
        id: row.get("id")?,
        app_id: row.get("app_id")?,
        contact: row.get("contact")?,
        direction: if direction == "in" {
            Direction::In
        } else {
            Direction::Out
        },
        kind: row.get("type")?,
        status: MessageStatus::parse(&row.get::<_, String>("status")?),
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        rendered: serde_json::from_str(&rendered).unwrap_or_else(|_| Rendered::of_kind("unknown")),
        issues: Issue::parse_all(&issues),
        error: json_column(row, "error")?,
        context_id: row.get("context_id")?,
        created_at: row.get("created_at")?,
    })
}

fn to_webhook(row: &Row<'_>) -> rusqlite::Result<Webhook> {
    Ok(Webhook {
        id: row.get("id")?,
        app_id: row.get("app_id")?,
        url: row.get("url")?,
        event: row.get("event")?,
        message_id: row.get("message_id")?,
        body: row.get("body")?,
        status_code: row.get("status_code")?,
        response: row.get("response")?,
        error: row.get("error")?,
        duration_ms: row.get("duration_ms")?,
        created_at: row.get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inbound(id: &str) -> Message {
        Message {
            id: id.to_owned(),
            app_id: "a".into(),
            contact: "15550000000".into(),
            direction: Direction::In,
            kind: "image".into(),
            status: MessageStatus::Received,
            payload: json!({}),
            rendered: Rendered::of_kind("image"),
            issues: Vec::new(),
            error: None,
            context_id: None,
            created_at: 1,
        }
    }

    fn media_at(store: &Store, id: &str, file: &str) -> Media {
        let path = store.media_dir().join(file);
        std::fs::write(&path, b"png").unwrap();

        Media {
            id: id.to_owned(),
            app_id: "a".into(),
            mime_type: "image/png".into(),
            filename: None,
            sha256: "x".into(),
            file_size: 3,
            path: path.to_string_lossy().into_owned(),
            created_at: 1,
        }
    }

    #[test]
    fn a_failed_inbound_media_save_leaves_no_message_and_no_file() {
        let data = tempfile::TempDir::new().unwrap();
        let store = Store::open(data.path(), Arc::new(crate::clock::SystemClock)).unwrap();
        let first = media_at(&store, "1", "first");
        store
            .record_message(&inbound("wamid.1"), Some("Mary"), Some(&first))
            .unwrap();

        let clashing = media_at(&store, "1", "second");
        assert!(
            store
                .record_message(&inbound("wamid.2"), None, Some(&clashing))
                .is_err()
        );

        assert!(store.message("wamid.2").unwrap().is_none());
        assert!(!std::path::Path::new(&clashing.path).exists());
        assert!(store.message("wamid.1").unwrap().is_some());
        assert_eq!(
            store.contact_name("a", "15550000000").unwrap().as_deref(),
            Some("Mary")
        );
    }

    #[test]
    fn a_failed_upload_save_removes_the_file() {
        let data = tempfile::TempDir::new().unwrap();
        let store = Store::open(data.path(), Arc::new(crate::clock::SystemClock)).unwrap();
        store
            .record_upload(&media_at(&store, "1", "first"))
            .unwrap();

        let clashing = media_at(&store, "1", "second");
        assert!(store.record_upload(&clashing).is_err());
        assert!(!std::path::Path::new(&clashing.path).exists());
    }
}
