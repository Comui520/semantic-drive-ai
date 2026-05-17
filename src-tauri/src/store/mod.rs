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
            );

            CREATE TABLE IF NOT EXISTS user_tag_history (
                name TEXT PRIMARY KEY,
                frequency INTEGER NOT NULL DEFAULT 1,
                last_used_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_user_tag_freq ON user_tag_history(frequency DESC);

            CREATE TABLE IF NOT EXISTS file_custom_tags (
                file_id TEXT NOT NULL,
                tag TEXT NOT NULL,
                PRIMARY KEY (file_id, tag)
            );

            CREATE INDEX IF NOT EXISTS idx_file_custom_tags_file ON file_custom_tags(file_id);

            CREATE TABLE IF NOT EXISTS chat_sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '新对话',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chat_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                file_refs TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_chat_messages_session ON chat_messages(session_id);",
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

    /// Get file entries by their IDs.
    pub fn get_files_by_ids(&self, ids: &[String]) -> Result<Vec<FileEntry>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let placeholders: Vec<String> = (0..ids.len()).map(|i| format!("?{}", i + 1)).collect();
        let sql = format!(
            "SELECT id, path, name, extension, mime_type, size, hash, modified, created, indexed_at \
             FROM files WHERE id IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| format!("Prepare error: {}", e))?;
        let params: Vec<Box<dyn rusqlite::types::ToSql>> =
            ids.iter().map(|s| Box::new(s.clone()) as Box<dyn rusqlite::types::ToSql>).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let entries = stmt
            .query_map(param_refs.as_slice(), |row| {
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

    /// Get extracted text content for a file by ID.
    pub fn get_content_text(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let result = conn.query_row(
            "SELECT content_text FROM files WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        );
        match result {
            Ok(text) => Ok(text),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Query error: {}", e)),
        }
    }

    /// Get a map of (path → (id, modified)) for all indexed files.
    /// Used by incremental scan to detect new/changed/deleted files.
    pub fn get_paths_map(&self) -> Result<std::collections::HashMap<String, (String, String)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT id, path, modified FROM files")
            .map_err(|e| format!("Query error: {}", e))?;
        let map = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    (row.get::<_, String>(0)?, row.get::<_, String>(2)?),
                ))
            })
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }

    /// Get a set of all known file paths.
    pub fn get_existing_paths(&self) -> Result<std::collections::HashSet<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT path FROM files")
            .map_err(|e| format!("Query error: {}", e))?;
        let paths = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(paths)
    }

    /// Insert or update a file entry (matched by path).
    pub fn upsert_file(&self, entry: &FileEntry) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;

        // Check if a file with this path already exists
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![entry.path],
                |row| row.get(0),
            )
            .ok();

        if let Some(old_id) = existing {
            conn.execute(
                "UPDATE files SET id=?1, name=?2, extension=?3, mime_type=?4, size=?5,
                 hash=?6, modified=?7, created=?8, indexed_at=?9
                 WHERE id=?10",
                params![
                    entry.id, entry.name, entry.extension, entry.mime_type,
                    entry.size, entry.hash, entry.modified, entry.created,
                    entry.indexed_at, old_id,
                ],
            )
            .map_err(|e| format!("Update error: {}", e))?;
        } else {
            conn.execute(
                "INSERT INTO files (id, path, name, extension, mime_type, size, hash, modified, created, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    entry.id, entry.path, entry.name, entry.extension, entry.mime_type,
                    entry.size, entry.hash, entry.modified, entry.created, entry.indexed_at,
                ],
            )
            .map_err(|e| format!("Insert error: {}", e))?;
        }
        Ok(())
    }

    /// Batch upsert: single lock, single transaction, one bulk query for existing paths.
    pub fn upsert_files_batch(&self, entries: &[FileEntry]) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction error: {}", e))?;

        // Collect all existing path→id mappings in one query
        let existing: std::collections::HashMap<String, String> = {
            let mut stmt = tx
                .prepare("SELECT path, id FROM files")
                .map_err(|e| format!("Query error: {}", e))?;
            let mut map = std::collections::HashMap::new();
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| format!("Query map error: {}", e))?;
            for row in rows {
                if let Ok((path, id)) = row {
                    map.insert(path, id);
                }
            }
            map
        };

        let mut count = 0;
        for entry in entries {
            if let Some(existing_id) = existing.get(&entry.path) {
                tx.execute(
                    "UPDATE files SET id=?1, name=?2, extension=?3, mime_type=?4, size=?5,
                     hash=?6, modified=?7, created=?8, indexed_at=?9 WHERE id=?10",
                    params![
                        entry.id, entry.name, entry.extension, entry.mime_type,
                        entry.size, entry.hash, entry.modified, entry.created,
                        entry.indexed_at, existing_id,
                    ],
                )
                .map_err(|e| format!("Update error: {}", e))?;
            } else {
                tx.execute(
                    "INSERT INTO files (id, path, name, extension, mime_type, size, hash, modified, created, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        entry.id, entry.path, entry.name, entry.extension, entry.mime_type,
                        entry.size, entry.hash, entry.modified, entry.created, entry.indexed_at,
                    ],
                )
                .map_err(|e| format!("Insert error: {}", e))?;
            }
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

    /// Delete a file entry by id.
    pub fn remove_file(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute("DELETE FROM files WHERE id = ?1", params![id])
            .map_err(|e| format!("Delete error: {}", e))?;
        Ok(())
    }

    /// Get files filtered by category (pre-classified).
    pub fn get_files_by_category(&self, category: &str) -> Result<Vec<FileEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, path, name, extension, mime_type, size, hash, modified, created, indexed_at
                 FROM files WHERE category = ?1 ORDER BY name",
            )
            .map_err(|e| format!("Query error: {}", e))?;
        let entries = stmt
            .query_map(params![category], |row| {
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

    /// Get files matching a tag (LIKE query on tags column).
    pub fn get_files_by_tag(&self, tag: &str) -> Result<Vec<FileEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let pattern = format!("%{}%", tag);
        let mut stmt = conn
            .prepare(
                "SELECT id, path, name, extension, mime_type, size, hash, modified, created, indexed_at
                 FROM files WHERE tags IS NOT NULL AND tags LIKE ?1 ORDER BY name",
            )
            .map_err(|e| format!("Query error: {}", e))?;
        let entries = stmt
            .query_map(params![pattern], |row| {
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

    /// Get aggregated category stats from existing DB data (for cache rebuild on restart).
    pub fn get_category_aggregates(&self) -> Result<Vec<(String, u64, u64)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT COALESCE(category, '其他') as cat, COUNT(*) as cnt, COALESCE(SUM(size), 0) as total FROM files GROUP BY cat ORDER BY cnt DESC")
            .map_err(|e| format!("Query error: {}", e))?;
        let results = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, u64>(2)?)))
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Get all non-empty tags strings (for cache rebuild on restart).
    pub fn get_all_tags(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT tags FROM files WHERE tags IS NOT NULL AND tags != ''")
            .map_err(|e| format!("Query error: {}", e))?;
        let tags = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tags)
    }

    /// Get all hashed entries for dedup cache rebuild (hash, path, name, size, modified).
    pub fn get_hash_entries(&self) -> Result<Vec<(String, String, String, u64, String)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT hash, path, name, size, modified FROM files WHERE hash IS NOT NULL")
            .map_err(|e| format!("Query error: {}", e))?;
        let entries = stmt
            .query_map([], |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, String>(4)?,
            )))
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    /// Delete file entries that no longer exist on disk.
    pub fn remove_missing_files(&self, valid_paths: &std::collections::HashSet<String>) -> Result<usize, String> {
        let existing = self.get_existing_paths()?;
        let to_remove: Vec<String> = existing.difference(valid_paths).cloned().collect();
        let count = to_remove.len();
        for path in &to_remove {
            let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
            conn.execute("DELETE FROM files WHERE path = ?1", params![path])
                .map_err(|e| format!("Delete error: {}", e))?;
        }
        Ok(count)
    }

    /// Record or update a user-applied tag (increment frequency, update timestamp).
    pub fn upsert_user_tag(&self, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO user_tag_history (name, frequency, last_used_at) VALUES (?1, 1, datetime('now'))
             ON CONFLICT(name) DO UPDATE SET frequency = frequency + 1, last_used_at = datetime('now')",
            params![name],
        )
        .map_err(|e| format!("Upsert user tag error: {}", e))?;
        Ok(())
    }

    /// Query user tags by prefix filter. Returns (name, frequency) sorted by frequency desc.
    pub fn get_user_tags(&self, filter: &str, limit: u32) -> Result<Vec<(String, u32)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let pattern = format!("{}%", filter);
        let mut stmt = conn
            .prepare(
                "SELECT name, frequency FROM user_tag_history WHERE name LIKE ?1 ORDER BY frequency DESC, last_used_at DESC LIMIT ?2",
            )
            .map_err(|e| format!("Query error: {}", e))?;
        let results = stmt
            .query_map(params![pattern, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })
            .map_err(|e| format!("Query map error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Get the most frequently used user tags.
    pub fn get_top_user_tags(&self, limit: u32) -> Result<Vec<(String, u32)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT name, frequency FROM user_tag_history ORDER BY frequency DESC, last_used_at DESC LIMIT ?1",
            )
            .map_err(|e| format!("Query error: {}", e))?;
        let results = stmt
            .query_map(params![limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })
            .map_err(|e| format!("Query map error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Set custom tags for a file (replaces all existing custom tags for this file).
    pub fn set_file_custom_tags(&self, file_id: &str, tags: &[String]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction error: {}", e))?;

        tx.execute("DELETE FROM file_custom_tags WHERE file_id = ?1", params![file_id])
            .map_err(|e| format!("Delete custom tags error: {}", e))?;

        for tag in tags {
            if !tag.trim().is_empty() {
                tx.execute(
                    "INSERT INTO file_custom_tags (file_id, tag) VALUES (?1, ?2)",
                    params![file_id, tag.trim()],
                )
                .map_err(|e| format!("Insert custom tag error: {}", e))?;
            }
        }

        tx.commit().map_err(|e| format!("Commit error: {}", e))?;
        Ok(())
    }

    /// Get custom tags for a file.
    pub fn get_file_custom_tags(&self, file_id: &str) -> Result<Vec<String>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT tag FROM file_custom_tags WHERE file_id = ?1 ORDER BY tag")
            .map_err(|e| format!("Query error: {}", e))?;
        let tags = stmt
            .query_map(params![file_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Query map error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tags)
    }

    /// Batch-get custom tags for multiple files. Single SQL query.
    pub fn get_files_custom_tags_batch(&self, file_ids: &[String]) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
        use rusqlite::types::ToSql;
        if file_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let placeholders: Vec<String> = (0..file_ids.len()).map(|i| format!("?{}", i + 1)).collect();
        let sql = format!(
            "SELECT file_id, tag FROM file_custom_tags WHERE file_id IN ({}) ORDER BY file_id, tag",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| format!("Prepare error: {}", e))?;
        let params: Vec<&dyn ToSql> = file_ids.iter().map(|s| s as &dyn ToSql).collect();
        let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| format!("Query error: {}", e))?;
        for row in rows {
            if let Ok((fid, tag)) = row {
                map.entry(fid).or_default().push(tag);
            }
        }
        Ok(map)
    }

    /// Get all custom tags with their file counts, sorted by count desc then name.
    pub fn get_all_tags_with_counts(&self) -> Result<Vec<(String, u32)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT tag, COUNT(*) as cnt FROM file_custom_tags GROUP BY tag ORDER BY cnt DESC, tag")
            .map_err(|e| format!("Prepare error: {}", e))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)))
            .map_err(|e| format!("Query error: {}", e))?;
        let mut result = Vec::new();
        for row in rows {
            if let Ok(r) = row {
                result.push(r);
            }
        }
        Ok(result)
    }

    /// Get file entries that have a specific custom tag.
    pub fn get_files_by_custom_tag(&self, tag: &str) -> Result<Vec<FileEntry>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT f.id, f.path, f.name, f.extension, f.mime_type, f.size, f.hash, f.modified, f.created, f.indexed_at
                 FROM files f INNER JOIN file_custom_tags t ON f.id = t.file_id
                 WHERE t.tag = ?1 ORDER BY f.path",
            )
            .map_err(|e| format!("Prepare error: {}", e))?;
        let entries = stmt
            .query_map(params![tag], |row| {
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

    /// Remove orphan tags from user_tag_history (tags that no longer appear in file_custom_tags).
    pub fn cleanup_orphan_tags(&self) -> Result<u32, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let deleted = conn
            .execute(
                "DELETE FROM user_tag_history WHERE name NOT IN (SELECT DISTINCT tag FROM file_custom_tags)",
                [],
            )
            .map_err(|e| format!("Delete error: {}", e))?;
        Ok(deleted as u32)
    }

    // ── Chat session / message methods ──

    /// Create a new chat session.
    pub fn create_chat_session(&self, id: &str, title: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chat_sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, title, now, now],
        )
        .map_err(|e| format!("Create session error: {}", e))?;
        Ok(())
    }

    /// List all chat sessions with message count, ordered by updated_at desc.
    pub fn list_chat_sessions(&self) -> Result<Vec<(String, String, String, String, u32)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.title, s.created_at, s.updated_at,
                        COALESCE((SELECT COUNT(*) FROM chat_messages m WHERE m.session_id = s.id), 0) as msg_count
                 FROM chat_sessions s ORDER BY s.updated_at DESC",
            )
            .map_err(|e| format!("Prepare error: {}", e))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u32>(4)?,
                ))
            })
            .map_err(|e| format!("Query error: {}", e))?;
        let mut result = Vec::new();
        for row in rows {
            if let Ok(r) = row {
                result.push(r);
            }
        }
        Ok(result)
    }

    /// Get a single chat session by id.
    pub fn get_chat_session(&self, id: &str) -> Result<Option<(String, String, String, String, u32)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let result = conn
            .query_row(
                "SELECT s.id, s.title, s.created_at, s.updated_at,
                        COALESCE((SELECT COUNT(*) FROM chat_messages m WHERE m.session_id = s.id), 0) as msg_count
                 FROM chat_sessions s WHERE s.id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, u32>(4)?,
                    ))
                },
            );
        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Query error: {}", e)),
        }
    }

    /// Update chat session title.
    pub fn update_chat_session_title(&self, id: &str, title: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE chat_sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, chrono::Utc::now().to_rfc3339(), id],
        )
        .map_err(|e| format!("Update title error: {}", e))?;
        Ok(())
    }

    /// Delete a chat session and all its messages.
    pub fn delete_chat_session(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute("DELETE FROM chat_messages WHERE session_id = ?1", params![id])
            .map_err(|e| format!("Delete messages error: {}", e))?;
        conn.execute("DELETE FROM chat_sessions WHERE id = ?1", params![id])
            .map_err(|e| format!("Delete session error: {}", e))?;
        Ok(())
    }

    /// Insert a chat message. Updates session's updated_at timestamp.
    pub fn insert_chat_message(
        &self,
        id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        file_refs_json: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chat_messages (id, session_id, role, content, file_refs, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, session_id, role, content, file_refs_json, now],
        )
        .map_err(|e| format!("Insert message error: {}", e))?;
        conn.execute(
            "UPDATE chat_sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )
        .map_err(|e| format!("Update session error: {}", e))?;
        Ok(())
    }

    /// Get all messages for a session, ordered by created_at.
    pub fn get_session_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<(String, String, String, String, Option<String>, String)>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, file_refs, created_at
                 FROM chat_messages WHERE session_id = ?1 ORDER BY created_at",
            )
            .map_err(|e| format!("Prepare error: {}", e))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|e| format!("Query error: {}", e))?;
        let mut result = Vec::new();
        for row in rows {
            if let Ok(r) = row {
                result.push(r);
            }
        }
        Ok(result)
    }
}

/// Get the path for app data directory on the storage device.
pub fn get_app_data_dir() -> Result<PathBuf, String> {
    crate::scanner::get_device_root().map(|p| p.join(".semanticdrive"))
}
