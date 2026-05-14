use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: String,
    pub path: String,
    pub name: String,
    pub extension: String,
    pub mime_type: String,
    pub size: u64,
    pub hash: Option<String>,
    pub modified: String,
    pub created: String,
    pub indexed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub files_found: u64,
    pub files_processed: u64,
    pub current_file: String,
    pub done: bool,
}

fn system_time_to_iso(time: std::time::SystemTime) -> String {
    let dt: DateTime<Utc> = time.into();
    dt.to_rfc3339()
}

fn guess_mime(path: &Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

/// Compute BLAKE3 hash of a file efficiently (streaming)
pub fn compute_hash(path: &Path) -> Option<String> {
    use std::fs::File;
    use std::io::Read;

    let file = File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => { hasher.update(&buffer[..n]); }
            Err(_) => return None,
        }
    }
    Some(hasher.finalize().to_hex().to_string())
}

/// Scan a directory recursively and return file entries.
/// `root` is the device mount root (absolute path).
/// `progress_cb` is called periodically with scan progress.
pub fn scan_directory(
    root: &Path,
) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    let root_canonical = root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve root path: {}", e))?;

    for entry in WalkDir::new(&root_canonical)
        .follow_links(false)
        .max_depth(20)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Skip hidden files and system files
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name.starts_with("$") {
                continue;
            }
        }

        // Skip our own index/cache directory
        if let Ok(rel) = path.strip_prefix(&root_canonical) {
            if rel.starts_with(".semanticdrive") {
                continue;
            }
        }

        let metadata = match path.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_size = metadata.len();
        let modified = metadata
            .modified()
            .map(system_time_to_iso)
            .unwrap_or_default();
        let created = metadata
            .created()
            .map(system_time_to_iso)
            .unwrap_or_default();
        let now = Utc::now().to_rfc3339();

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime = guess_mime(path);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let rel_path = path
            .strip_prefix(&root_canonical)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string()
            .replace('\\', "/");

        let id = blake3::hash(rel_path.as_bytes()).to_hex()[..16].to_string();

        entries.push(FileEntry {
            id,
            path: rel_path,
            name,
            extension: ext,
            mime_type: mime,
            size: file_size,
            hash: None,
            modified,
            created,
            indexed_at: now,
        });
    }

    Ok(entries)
}

/// Get the root path of the storage device (directory where executable resides)
pub fn get_device_root() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| format!("Cannot determine executable path: {}", e))
        .and_then(|exe_path| {
            exe_path
                .parent()
                .ok_or_else(|| "No parent directory".to_string())
                .map(|p| p.to_path_buf())
        })
}
