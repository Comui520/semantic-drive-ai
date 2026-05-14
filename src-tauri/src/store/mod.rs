use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::scanner::FileEntry;

pub struct MetadataStore {
    conn: Mutex<Connection>,
}

impl MetadataStore {
    /// Open or create the metadata database at the given directory.
    pub fn open(db_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(db_dir)
            .map_err(|e| format!("Cannot create db dir: {}", e))?;

        let db_path = db_dir.join("metadata.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Cannot open database: {}", e))?;

        // Enable WAL mode for better concurrent performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("Cannot set WAL mode: {}", e))?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.initialize_tables()?;
        Ok(store)
    }

    fn initialize_tables(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                extension TEXT,
                mime_type TEXT,
                size INTEGER NOT NULL,
                hash TEXT,
                modified TEXT,
                created TEXT,
                indexed_at TEXT NOT NULL,
                content_text TEXT,
                tags TEXT,
                category TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_name ON files(name);
            CREATE INDEX IF NOT EXISTS idx_files_extension ON files(extension);
            CREATE INDEX IF NOT EXISTS idx_files_category ON files(category);
            CREATE INDEX IF NOT EXISTS idx_files_tags ON files(tags);

            CREATE TABLE IF NOT EXISTS scan_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                root_path TEXT,
                last_scan TEXT,
                total_files INTEGER
            );",
        )
        .map_err(|e| format!("Cannot create tables: {}", e))?;
        Ok(())
    }

    /// Replace all file entries with a new set (full reindex).
    pub fn replace_all(&self, entries: &[FileEntry]) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction error: {}", e))?;

        tx.execute("DELETE FROM files", [])
            .map_err(|e| format!("Delete error: {}", e))?;

        let mut count = 0;
        for entry in entries {
            tx.execute(
                "INSERT INTO files (id, path, name, extension, mime_type, size, hash, modified, created, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    entry.id,
                    entry.path,
                    entry.name,
                    entry.extension,
                    entry.mime_type,
                    entry.size,
                    entry.hash,
                    entry.modified,
                    entry.created,
                    entry.indexed_at,
                ],
            )
            .map_err(|e| format!("Insert error: {}", e))?;
            count += 1;
        }

        tx.execute(
            "INSERT OR REPLACE INTO scan_state (id, root_path, last_scan, total_files) VALUES (1, '', datetime('now'), ?1)",
            params![count],
        )
        .map_err(|e| format!("Scan state error: {}", e))?;

        tx.commit().map_err(|e| format!("Commit error: {}", e))?;
        Ok(count)
    }

    /// Get all files in the database.
    pub fn get_all_files(&self) -> Result<Vec<FileEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, path, name, extension, mime_type, size, hash, modified, created, indexed_at
                 FROM files ORDER BY name",
            )
            .map_err(|e| format!("Query error: {}", e))?;

        let entries = stmt
            .query_map([], |row| {
                Ok(FileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    name: row.get(2)?,
                    extension: row.get(3)?,
                    mime_type: row.get(4)?,
                    size: row.get(5)?,
                    hash: row.get(6)?,
                    modified: row.get(7)?,
                    created: row.get(8)?,
                    indexed_at: row.get(9)?,
                })
            })
            .map_err(|e| format!("Query map error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    /// Get total file count.
    pub fn file_count(&self) -> Result<u64, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get::<_, u64>(0))
            .map_err(|e| format!("Count error: {}", e))
    }

    /// Update hash for a file.
    pub fn update_hash(&self, id: &str, hash: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute("UPDATE files SET hash = ?1 WHERE id = ?2", params![hash, id])
            .map_err(|e| format!("Update hash error: {}", e))?;
        Ok(())
    }

    /// Update category for a file.
    pub fn update_category(&self, id: &str, category: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE files SET category = ?1 WHERE id = ?2",
            params![category, id],
        )
        .map_err(|e| format!("Update category error: {}", e))?;
        Ok(())
    }

    /// Add tags to a file.
    pub fn update_tags(&self, id: &str, tags: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE files SET tags = ?1 WHERE id = ?2",
            params![tags, id],
        )
        .map_err(|e| format!("Update tags error: {}", e))?;
        Ok(())
    }

    /// Update extracted text content for a file.
    pub fn update_content(&self, id: &str, content: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE files SET content_text = ?1 WHERE id = ?2",
            params![content, id],
        )
        .map_err(|e| format!("Update content error: {}", e))?;
        Ok(())
    }
}

/// Get the path for app data directory on the storage device.
pub fn get_app_data_dir() -> Result<PathBuf, String> {
    crate::scanner::get_device_root().map(|p| p.join(".semanticdrive"))
}
