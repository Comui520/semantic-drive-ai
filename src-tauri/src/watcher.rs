use crate::{ai, scanner, AppState};
use notify::Event;
use std::path::PathBuf;
use tauri::{Emitter, Manager};

/// Incrementally synchronize one filesystem event with SQLite, extracted
/// content, and the in-memory local search index.
pub(crate) fn sync_file_event(app: &tauri::AppHandle, event: Event, root: PathBuf) {
    use notify::EventKind;
    let Some(path) = event.paths.first().cloned() else { return; };
    let kind_name = format!("{:?}", event.kind);
    let app_bg = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_bg.state::<AppState>();
        let is_remove = matches!(event.kind, EventKind::Remove(_));
        let relative = path.strip_prefix(&root).ok().map(|p| p.to_string_lossy().replace('\\', "/"));
        if let Some(relative) = relative {
            if relative.starts_with(".semanticdrive/") || relative == ".semanticdrive" { return; }
            let mut synced = false;
            if !is_remove {
                if let Ok(Some(mut entry)) = scanner::scan_single_file(&root, &path) {
                    if entry.size <= 100 * 1024 * 1024 { entry.hash = scanner::compute_hash(&path); }
                    if let Ok(guard) = state.store.lock() {
                        if let Some(db) = guard.as_ref() {
                            let _ = db.upsert_file(&entry);
                            if let Ok(text) = ai::extract_text(&path, &entry.extension) {
                                if text.len() > 20 {
                                    let _ = db.update_content(&entry.id, &text);
                                    if let Ok(mut engine) = state.search_engine.write() { engine.index_file(&entry.id, &text); }
                                }
                            }
                            synced = true;
                        }
                    }
                }
            } else if let Ok(guard) = state.store.lock() {
                if let Some(db) = guard.as_ref() {
                    let _ = db.remove_file_by_path(&relative);
                    let id = blake3::hash(relative.as_bytes()).to_hex()[..16].to_string();
                    if let Ok(mut engine) = state.search_engine.write() { engine.remove_file(&id); }
                    synced = true;
                }
            }
            let _ = app_bg.emit("files-changed", serde_json::json!({
                "kind": kind_name, "path": relative, "synced": synced
            }));
        }
    });
}
