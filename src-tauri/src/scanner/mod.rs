use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;
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
/// Recursively scan a directory, collecting file metadata.
/// Skips hidden files, system files, and files under `exclude_dir` (if Some).
pub fn scan_directory(
    root: &Path,
    exclude_dir: Option<&Path>,
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

        // Skip files under the excluded directory (e.g. app's own folder)
        if let Some(exclude) = exclude_dir {
            if let Ok(exclude_canonical) = exclude.canonicalize() {
                if path.starts_with(&exclude_canonical) {
                    continue;
                }
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


/// Build metadata for one filesystem path. Used by the watcher to update the
/// index without rescanning the entire workspace.
pub fn scan_single_file(root: &Path, path: &Path) -> Result<Option<FileEntry>, String> {
    let canonical_root = root.canonicalize().map_err(|e| format!("Cannot resolve root path: {}", e))?;
    let canonical_path = path.canonicalize().map_err(|e| format!("Cannot resolve file path: {}", e))?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() { return Ok(None); }
    let relative = canonical_path.strip_prefix(&canonical_root).map_err(|_| "File is outside workspace".to_string())?;
    if relative.components().any(|part| part.as_os_str() == ".semanticdrive") { return Ok(None); }
    let metadata = std::fs::metadata(&canonical_path).map_err(|e| format!("Cannot read file metadata: {}", e))?;
    let name = canonical_path.file_name().and_then(|v| v.to_str()).unwrap_or("unknown").to_string();
    if name.starts_with('.') || name.starts_with('$') { return Ok(None); }
    let rel_path = relative.to_string_lossy().replace('\\', "/");
    let modified = metadata.modified().map(system_time_to_iso).unwrap_or_default();
    let created = metadata.created().map(system_time_to_iso).unwrap_or_default();
    Ok(Some(FileEntry {
        id: blake3::hash(rel_path.as_bytes()).to_hex()[..16].to_string(),
        path: rel_path,
        name,
        extension: canonical_path.extension().and_then(|v| v.to_str()).unwrap_or("").to_lowercase(),
        mime_type: guess_mime(&canonical_path),
        size: metadata.len(),
        hash: None,
        modified,
        created,
        indexed_at: Utc::now().to_rfc3339(),
    }))
}

/// Get the root path of the storage device (directory where executable resides).
/// Used for `.semanticdrive/` internal paths (models, DB, cache).
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

static SCAN_ROOT_OVERRIDE: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn scan_root_override() -> &'static RwLock<Option<PathBuf>> {
    SCAN_ROOT_OVERRIDE.get_or_init(|| RwLock::new(None))
}

/// Set the user-selected workspace that the scanner and Agent are allowed to use.
pub fn set_scan_root_override(path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    let canonical = path.map(|value| {
        if !value.is_dir() {
            return Err(format!("扫描目录不存在或不是目录: {}", value.display()));
        }
        std::fs::canonicalize(&value).map_err(|e| format!("无法解析扫描目录: {e}"))
    }).transpose()?;
    *scan_root_override().write().map_err(|e| e.to_string())? = canonical.clone();
    Ok(canonical)
}

/// Get the user-selected scan root. For backwards compatibility, an app beside
/// a removable drive still falls back to the legacy executable-derived root.
pub fn get_scan_root() -> Result<PathBuf, String> {
    if let Some(root) = scan_root_override().read().map_err(|e| e.to_string())?.clone() {
        return Ok(root);
    }
    let exe_dir = get_device_root()?;
    // Go one level up from exe directory to scan the containing folder/drive root
    match exe_dir.parent() {
        Some(parent) => Ok(parent.to_path_buf()),
        None => Ok(exe_dir), // already at root, can't go up
    }
}

/// Start a polling filesystem watcher that checks for changes every 2 seconds.
/// The callback is called on a background thread for each relevant event.
/// Returns a `PollWatcher` which must be kept alive for the app's lifetime.
pub fn start_file_watcher(
    root: PathBuf,
    on_event: impl Fn(notify::Event) + Send + 'static,
) -> Result<notify::PollWatcher, String> {
    use notify::{Config, RecursiveMode, Watcher};

    let mut watcher = notify::PollWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if is_relevant_event(&event) {
                    on_event(event);
                }
            }
        },
        Config::default().with_poll_interval(Duration::from_secs(2)),
    )
    .map_err(|e| format!("Watcher creation failed: {}", e))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("Watch failed: {}", e))?;

    Ok(watcher)
}

/// Filter events for hidden files, system files, and our own cache directory.
fn is_relevant_event(event: &notify::Event) -> bool {
    !event.paths.iter().any(|p| {
        p.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s.starts_with('.') || s.starts_with('$') || s == ".semanticdrive"
        })
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("semantic-drive-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn scan_single_file_normalizes_relative_paths() {
        let root = temp_workspace("scanner");
        let nested = root.join("docs");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("报告.txt");
        fs::write(&file, "hello").unwrap();
        let entry = scan_single_file(&root, &file).unwrap().unwrap();
        assert_eq!(entry.path, "docs/报告.txt");
        assert_eq!(entry.extension, "txt");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scan_single_file_rejects_paths_outside_root() {
        let root = temp_workspace("root");
        let outside = temp_workspace("outside").join("secret.txt");
        fs::write(&outside, "secret").unwrap();
        assert!(scan_single_file(&root, &outside).unwrap().is_none());
        let _ = fs::remove_dir_all(root.parent().unwrap_or(&root));
        let _ = fs::remove_dir_all(outside.parent().unwrap_or(&outside));
    }
}
