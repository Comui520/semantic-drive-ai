mod ai;
mod chat;
mod classifier;
mod dedup;
mod scanner;
mod store;
mod vault;

use ai::llm::{LlmEngine, ParsedQuery, GenerateConfig};
use ai::model_manager::ModelInfo;
use ai::search::{SearchEngine, SearchResult};
use classifier::{CategoryInfo, ClassificationResult, TagInfo};
use dedup::DuplicateGroup;
use scanner::FileEntry;
use store::MetadataStore;
use chat::{ChatSession, ChatMessage, ChatTokenEvent, ChatIntent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::RwLock;
use tauri::{Emitter, Manager, State};

struct AppState {
    store: Mutex<Option<MetadataStore>>,
    device_root: Mutex<Option<String>>,
    search_engine: RwLock<SearchEngine>,
    llm_engine: Mutex<LlmEngine>,
    scan_progress: Mutex<Option<scanner::ScanProgress>>,
    classification_cache: Mutex<Option<ClassificationResult>>,
    duplicate_cache: Mutex<Option<Vec<DuplicateGroup>>>,
    search_cancelled: Mutex<bool>,
    chat_cancelled: AtomicBool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(&entry.path(), &dst.join(entry.file_name())).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ── Device & Scanning Commands ──

#[tauri::command]
fn get_device_root(state: State<AppState>) -> Result<String, String> {
    let root_lock = state.device_root.lock().map_err(|e| e.to_string())?;
    root_lock
        .clone()
        .ok_or_else(|| "Device root not initialized".to_string())
}

#[tauri::command]
async fn scan_files(app: tauri::AppHandle) -> Result<Vec<FileEntry>, String> {
    let scan_root = scanner::get_scan_root()
        .map_err(|e| format!("Cannot get scan root: {}", e))?;
    let device_root = scanner::get_device_root()
        .map_err(|e| format!("Cannot get device root: {}", e))?;

    {
        let state = app.state::<AppState>();
        let mut root_lock = state.device_root.lock().map_err(|e| e.to_string())?;
        *root_lock = Some(scan_root.to_string_lossy().to_string());
    }

    // Helper: update both event and shared state
    let set_progress = |app: &tauri::AppHandle, prog: scanner::ScanProgress| {
        let _ = app.emit("scan-progress", prog.clone());
        if let Ok(mut sp) = app.state::<AppState>().scan_progress.lock() {
            *sp = Some(prog);
        }
    };

    // Phase 1: Fast file listing
    let scan_root_cb = scan_root.clone();
    let exclude_cb = device_root.clone();
    let app_p1 = app.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let entries = scanner::scan_directory(&scan_root_cb, Some(&exclude_cb))?;
        let _ = app_p1.emit("scan-progress", scanner::ScanProgress {
            files_found: entries.len() as u64, files_processed: 0,
            current_file: String::new(), done: false,
        });
        Ok::<Vec<FileEntry>, String>(entries)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;
    set_progress(&app, scanner::ScanProgress {
        files_found: entries.len() as u64, files_processed: 0,
        current_file: "phase1 done".into(), done: false,
    });

    // Phase 2: DB diff (incremental — use modification time)
    let data_dir = device_root.join(".semanticdrive");
    let db = MetadataStore::open(&data_dir)?;
    let paths_map = db.get_paths_map().unwrap_or_default();
    let current_paths: std::collections::HashSet<String> = entries.iter().map(|e| e.path.clone()).collect();

    let mut new_entries: Vec<FileEntry> = Vec::new();
    let mut changed_entries: Vec<FileEntry> = Vec::new();
    for e in &entries {
        match paths_map.get(&e.path) {
            None => new_entries.push(e.clone()),
            Some((_id, modified)) if *modified != e.modified => changed_entries.push(e.clone()),
            _ => {}
        }
    }
    let need_indexing: Vec<FileEntry> = new_entries.iter().chain(changed_entries.iter()).cloned().collect();
    let removed_count = db.remove_missing_files(&current_paths).unwrap_or(0);
    log::info!("New: {}, Changed: {}, Removed: {}, Total: {}",
        new_entries.len(), changed_entries.len(), removed_count, entries.len());

    // Phase 4: Batch DB upsert (only new + changed)
    let entries_to_upsert = need_indexing.clone();
    let app_db = app.clone();
    set_progress(&app, scanner::ScanProgress {
        files_found: entries.len() as u64, files_processed: (entries.len() as u64).saturating_sub(1),
        current_file: "saving DB...".into(), done: false,
    });
    tokio::task::spawn_blocking(move || {
        if !entries_to_upsert.is_empty() {
            db.upsert_files_batch(&entries_to_upsert)?;
        }
        let state = app_db.state::<AppState>();
        if let Ok(mut store_lock) = state.store.lock() { *store_lock = Some(db); }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("DB panicked: {}", e))??;
    set_progress(&app, scanner::ScanProgress {
        files_found: entries.len() as u64, files_processed: entries.len() as u64,
        current_file: "indexing content...".into(), done: false,
    });

    // Phase 5a (parallel with Phase 3): Background classification
    if !entries.is_empty() {
        let app_5a = app.clone();
        let entries_5a = entries.clone();
        tokio::spawn(async move {
            let _ = app_5a.emit("classify-progress", serde_json::json!({
                "status": "classifying", "current": 0, "total": 0
            }));
            let app_bg = app_5a.clone();
            match tokio::task::spawn_blocking(move || -> Result<ClassificationResult, String> {
                let state = app_bg.state::<AppState>();
                let all_files = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
                    Some(db) => db.get_all_files().map_err(|e| e.to_string())?,
                    None => entries_5a,
                };
                let (result, cat_map, tag_map) = {
                    let engine = state.search_engine.read().map_err(|e| e.to_string())?;
                    classifier::classify_files_batch(&all_files, engine.embedding_engine())
                };
                let _ = app_bg.emit("classify-progress", serde_json::json!({
                    "status": "saving", "current": 0, "total": 0
                }));
                if !cat_map.is_empty() || !tag_map.is_empty() {
                    if let Some(db) = state.store.lock().map_err(|e| e.to_string())?.as_ref() {
                        for (id, cat) in &cat_map { let _ = db.update_category(id, cat); }
                        for (id, tags) in &tag_map { if !tags.is_empty() { let _ = db.update_tags(id, tags); } }
                    }
                }
                Ok(result)
            }).await {
                Ok(Ok(result)) => {
                    let _ = app_5a.emit("classify-progress", serde_json::json!({
                        "status": "done", "current": 1, "total": 1
                    }));
                    if let Ok(mut cache) = app_5a.state::<AppState>().classification_cache.lock() {
                        *cache = Some(result);
                    }
                }
                Ok(Err(e)) => {
                    log::error!("Background classification failed: {}", e);
                    let _ = app_5a.emit("classify-progress", serde_json::json!({
                        "status": "error", "error": e
                    }));
                }
                Err(e) => {
                    log::error!("Background classification task panicked: {}", e);
                    let _ = app_5a.emit("classify-progress", serde_json::json!({
                        "status": "error", "error": format!("Task panicked: {}", e)
                    }));
                }
            }
        });
    }

    // Phase 3: Extract + index new/changed files
    if !need_indexing.is_empty() {
        use rayon::prelude::*;
        let scan_root_idx = scan_root.clone();

        // 3a. Parallel text extraction
        let extracted: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
            Ok::<_, String>(
                need_indexing.par_iter().filter_map(|entry| {
                    let full_path = scan_root_idx.join(&entry.path);
                    if let Ok(meta) = std::fs::metadata(&full_path) {
                        if meta.len() > 100 * 1024 * 1024 { return None; }
                    }
                    match ai::extract_text(&full_path, &entry.extension) {
                        Ok(text) if text.len() > 20 => Some((entry.id.clone(), text)),
                        _ => None,
                    }
                }).collect()
            )
        })
        .await
        .map_err(|e| format!("Extraction panicked: {}", e))??;

        // Persist extracted content to DB for future searches (avoid re-extraction)
        if !extracted.is_empty() {
            if let Ok(store_guard) = app.state::<AppState>().store.lock() {
                if let Some(db) = store_guard.as_ref() {
                    for (id, text) in &extracted {
                        let _ = db.update_content(id, text);
                    }
                }
            }
        }

        set_progress(&app, scanner::ScanProgress {
            files_found: entries.len() as u64,
            files_processed: extracted.len() as u64,
            current_file: format!("extracted {}, embedding...", extracted.len()),
            done: false,
        });

        // 3b. Batch embedding with per-batch progress
        if !extracted.is_empty() {
            let app_idx = app.clone();
            let total_emb = extracted.len();
            tokio::task::spawn_blocking(move || {
                let state = app_idx.state::<AppState>();
                let mut all: Vec<(String, String, Vec<f32>, Option<Vec<f32>>)> = Vec::with_capacity(total_emb);
                let texts: Vec<String> = extracted.iter().map(|(_, t)| t.clone()).collect();

                for (chunk_idx, chunk) in texts.chunks(32).enumerate() {
                    // Progress before each batch
                    if let Ok(mut sp) = app_idx.state::<AppState>().scan_progress.lock() {
                        *sp = Some(scanner::ScanProgress {
                            files_found: total_emb as u64,
                            files_processed: all.len() as u64,
                            current_file: format!("embedding {}/{}", all.len(), total_emb),
                            done: false,
                        });
                    }

                    // Embed batch — acquire read lock, embed both zh + en, release
                    let (batch_embs_zh, batch_embs_en) = {
                        let engine = state.search_engine.read().map_err(|e| e.to_string())?;
                        let zh_embs = engine.embedding_engine().embed_batch(chunk);
                        let en_embs = engine.en_embedding_engine()
                            .map(|e| e.embed_batch(chunk))
                            .unwrap_or_default();
                        (zh_embs, en_embs)
                    };

                    let start = chunk_idx * 32;
                    for (i, zh_emb) in batch_embs_zh.into_iter().enumerate() {
                        let idx = start + i;
                        if idx < total_emb {
                            let (ref id, ref text) = extracted[idx];
                            let en_emb = batch_embs_en.get(i).cloned();
                            all.push((id.clone(), text.clone(), zh_emb, en_emb));
                        }
                    }
                }

                // Batch-inject into search engine (write lock, milliseconds)
                if !all.is_empty() {
                    let mut search = state.search_engine.write().map_err(|e| e.to_string())?;
                    search.index_files_batch(&all);
                }
                Ok::<(), String>(())
            })
            .await
            .map_err(|e| format!("Indexing panicked: {}", e))??;
        }
    }


    // Phase 5b: Background dedup (after Phase 3)
    if !entries.is_empty() {
        let app_5b = app.clone();
        let entries_5b = entries.clone();
        tokio::spawn(async move {
            let _ = app_5b.emit("dedup-progress", serde_json::json!({
                "status": "hashing", "current": 0, "total": 0
            }));
            if let Ok(root) = scanner::get_scan_root() {
                let groups = tokio::task::spawn_blocking(move || {
                    dedup::find_duplicates(&entries_5b, &root)
                }).await.unwrap_or_default();
                let _ = app_5b.emit("dedup-progress", serde_json::json!({
                    "status": "done", "current": 1, "total": 1
                }));
                if let Ok(mut cache) = app_5b.state::<AppState>().duplicate_cache.lock() {
                    *cache = Some(groups);
                }
            }
        });
    }

    // Done
    set_progress(&app, scanner::ScanProgress {
        files_found: entries.len() as u64, files_processed: entries.len() as u64,
        current_file: String::new(), done: true,
    });
    if let Ok(mut sp) = app.state::<AppState>().scan_progress.lock() { *sp = None; }
    Ok(entries)
}


#[tauri::command]
fn get_files(state: State<AppState>) -> Result<Vec<FileEntry>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_all_files(),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                let db = MetadataStore::open(&data_dir)?;
                db.get_all_files()
            } else {
                Ok(Vec::new())
            }
        }
    }
}

#[tauri::command]
fn get_scan_progress(state: State<AppState>) -> Result<Option<scanner::ScanProgress>, String> {
    let guard = state.scan_progress.lock().map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
fn get_file_count(state: State<AppState>) -> Result<u64, String> {
    // Quick check: release lock immediately after reading
    let count = state.store.lock().map_err(|e| e.to_string())?
        .as_ref().and_then(|db| db.file_count().ok());
    match count {
        Some(c) => Ok(c),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.file_count()
            } else {
                Ok(0)
            }
        }
    }
}

#[tauri::command]
fn compute_file_hash(file_path: String) -> Result<String, String> {
    let root = scanner::get_scan_root()?;
    let full_path = root.join(&file_path);
    scanner::compute_hash(&full_path)
        .ok_or_else(|| format!("Cannot hash file: {}", file_path))
}

// ── Search Commands ──

/// Merge multiple ParsedQuery results by taking the union of keywords/file_types/entities
/// and the intersection of time ranges.
fn merge_parsed_queries(queries: &[ParsedQuery]) -> ParsedQuery {
    let mut keywords: Vec<String> = Vec::new();
    let mut file_types: Vec<String> = Vec::new();
    let mut entities: Vec<String> = Vec::new();
    let mut merged_time: Option<(String, String)> = None;

    for q in queries {
        for kw in &q.keywords {
            if !keywords.contains(kw) {
                keywords.push(kw.clone());
            }
        }
        for ft in &q.file_types {
            if !file_types.contains(ft) {
                file_types.push(ft.clone());
            }
        }
        for e in &q.entities {
            if !entities.contains(e) {
                entities.push(e.clone());
            }
        }
        // Intersect time ranges: empty string = unbounded
        merged_time = match (merged_time.take(), &q.time_range) {
            (None, Some(r)) => Some(r.clone()),
            (Some(r), None) => Some(r),
            (Some((s1, e1)), Some((s2, e2))) => {
                let start = if s1.is_empty() {
                    s2.clone()
                } else if s2.is_empty() {
                    s1
                } else {
                    std::cmp::max(s1, s2.clone())
                };
                let end = if e1.is_empty() {
                    e2.clone()
                } else if e2.is_empty() {
                    e1
                } else {
                    std::cmp::min(e1, e2.clone())
                };
                Some((start, end))
            }
            (None, None) => None,
        };
    }

    ParsedQuery {
        keywords,
        time_range: merged_time,
        file_types,
        entities,
        original: String::new(),
    }
}

#[tauri::command]
fn cancel_search(state: State<AppState>) -> Result<(), String> {
    let mut flag = state.search_cancelled.lock().map_err(|e| e.to_string())?;
    *flag = true;
    Ok(())
}

#[tauri::command]
async fn search_files(
    app: tauri::AppHandle,
    query: String,
    tags: Vec<String>,
    max_results: usize,
    bilingual: bool,
) -> Result<Vec<SearchResult>, String> {
    // Reset cancellation flag at start
    {
        let state = app.state::<AppState>();
        let mut flag = state.search_cancelled.lock().map_err(|e| e.to_string())?;
        *flag = false;
    }

    let app_clone = app.clone();
    let query_clone = query.clone();
    let tags_clone = tags.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        let state = app_clone.state::<AppState>();
        let mut llm = state.llm_engine.lock().map_err(|e| e.to_string())?;

        // Parse the main query
        let main_parsed = llm.parse_query(&query_clone);

        // Parse each tag independently and merge
        if tags_clone.is_empty() {
            Ok::<_, String>(main_parsed)
        } else {
            let mut all = vec![main_parsed];
            for tag in &tags_clone {
                all.push(llm.parse_query(tag));
            }
            Ok(merge_parsed_queries(&all))
        }
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    // Check cancellation before proceeding to expensive search phase
    if *app.state::<AppState>().search_cancelled.lock().map_err(|e| e.to_string())? {
        return Ok(Vec::new());
    }

    let has_keywords = !parsed.keywords.is_empty();
    let has_time = parsed.time_range.is_some();
    let search_query = if has_keywords { parsed.keywords.join(" ") } else { query.clone() };

    let app_clone = app.clone();
    let search_query_clone = search_query.clone();
    let time_range = parsed.time_range.clone();
    let file_types = parsed.file_types.clone();
    let entities = parsed.entities.clone();
    let use_en = bilingual;
    let max_results_clamped = max_results.max(20).min(200);

    let results = tokio::task::spawn_blocking(move || {
        // Check cancellation inside the blocking task too
        if *app_clone.state::<AppState>().search_cancelled.lock().map_err(|e| e.to_string())? {
            return Ok::<Vec<SearchResult>, String>(Vec::new());
        }

        let state = app_clone.state::<AppState>();
        let engine = state.search_engine.read().map_err(|e| e.to_string())?;
        let files = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
            Some(db) => db.get_all_files().unwrap_or_default(),
            None => Vec::new(),
        };
        let mut results: Vec<SearchResult> = if has_keywords || !has_time {
            engine.search(&search_query_clone, &files, max_results_clamped * 2, use_en)
        } else {
            files.iter().map(|f| SearchResult {
                file_id: f.id.clone(), file_name: f.name.clone(), file_path: f.path.clone(),
                score: 0.5, match_type: "时间浏览".to_string(), snippet: String::new(),
                file_size: f.size, modified: f.modified.clone(),
            }).collect()
        };
        if let Some((ref start, ref end)) = time_range {
            results.retain(|r| r.modified >= *start && r.modified <= *end);
        }
        if !file_types.is_empty() {
            results.retain(|r| file_types.iter().any(|t| r.file_path.ends_with(t) || r.file_name.to_lowercase().ends_with(t)));
        }
        for r in &mut results {
            for entity in &entities {
                if r.file_name.contains(entity) || r.snippet.contains(entity) {
                    r.score = (r.score + 0.2).min(1.0);
                    r.match_type = "语义+实体匹配".to_string();
                }
            }
        }
        if !has_keywords && has_time {
            results.sort_by(|a, b| b.modified.cmp(&a.modified));
        } else {
            results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }
        results.truncate(max_results_clamped);
        Ok::<Vec<SearchResult>, String>(results)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    Ok(results)
}

// ── File Operations ──

#[tauri::command]
fn open_file_location(file_path: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let full_path = root.join(&file_path);
    if !full_path.exists() { return Err(format!("File not found: {}", full_path.display())); }

    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("explorer")
            .arg(format!("/select,{}", full_path.display()))
            .spawn()
            .map(|_| ()).map_err(|e| format!("Failed: {}", e));
    }

    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("open")
            .arg("-R")
            .arg(&full_path)
            .spawn()
            .map(|_| ()).map_err(|e| format!("Failed: {}", e));
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let parent = full_path.parent()
            .ok_or_else(|| "Cannot determine parent directory".to_string())?;
        return open::that(parent).map_err(|e| format!("Failed: {}", e));
    }
}

#[tauri::command]
fn open_file(file_path: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let full_path = root.join(&file_path);
    if !full_path.exists() { return Err(format!("File not found: {}", full_path.display())); }
    open::that(full_path).map_err(|e| format!("Failed: {}", e))
}

#[tauri::command]
fn open_folder(folder_path: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let full_path = root.join(&folder_path);
    if !full_path.exists() { return Err(format!("Folder not found: {}", full_path.display())); }
    open::that(full_path).map_err(|e| format!("Failed: {}", e))
}

#[tauri::command]
fn get_directory(dir_path: String) -> Result<Vec<DirEntry>, String> {
    let root = scanner::get_scan_root()?;
    let target = root.join(&dir_path);
    if !target.is_dir() {
        return Err(format!("Not a directory: {}", dir_path));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&target).map_err(|e| e.to_string())? {
        let e = entry.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        let meta = e.metadata().map_err(|e| e.to_string())?;
        let rel_path = if dir_path.is_empty() || dir_path == "." {
            name.clone()
        } else {
            let dir = dir_path.trim_end_matches('/').trim_end_matches('\\');
            format!("{}/{}", dir, name)
        };
        let modified = meta.modified()
            .map(|t| -> String { let dt: chrono::DateTime<chrono::Utc> = t.into(); dt.to_rfc3339() })
            .unwrap_or_default();
        entries.push(DirEntry {
            name,
            path: rel_path,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

#[tauri::command]
fn rename_file(old_path: String, new_name: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let full_old = root.join(&old_path);
    if !full_old.exists() {
        return Err(format!("File not found: {}", old_path));
    }
    let full_new = full_old.with_file_name(&new_name);
    std::fs::rename(&full_old, &full_new).map_err(|e| format!("Rename failed: {}", e))
}

#[tauri::command]
fn delete_file(file_path: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let full = root.join(&file_path);
    if !full.exists() {
        return Err(format!("File not found: {}", file_path));
    }
    if full.is_dir() {
        std::fs::remove_dir_all(&full).map_err(|e| format!("Delete failed: {}", e))
    } else {
        std::fs::remove_file(&full).map_err(|e| format!("Delete failed: {}", e))
    }
}

#[tauri::command]
fn move_file(source: String, destination: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let src = root.join(&source);
    let dst = root.join(&destination);
    if !src.exists() {
        return Err(format!("Source not found: {}", source));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create directory: {}", e))?;
    }
    std::fs::rename(&src, &dst).map_err(|e| format!("Move failed: {}", e))
}

#[tauri::command]
fn copy_file(source: String, destination: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let src = root.join(&source);
    let dst = root.join(&destination);
    if !src.exists() {
        return Err(format!("Source not found: {}", source));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create directory: {}", e))?;
    }
    if src.is_dir() {
        copy_dir_recursive(&src, &dst)
    } else {
        std::fs::copy(&src, &dst).map_err(|e| format!("Copy failed: {}", e))?;
        Ok(())
    }
}

#[tauri::command]
fn import_file(source: String, destination: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let src = std::path::PathBuf::from(&source);
    if !src.exists() {
        return Err(format!("Source not found: {}", source));
    }
    let dst = root.join(&destination);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create directory: {}", e))?;
    }
    std::fs::copy(&src, &dst).map_err(|e| format!("Copy failed: {}", e))?;
    Ok(())
}

#[tauri::command]
fn index_file_content(file_id: String, content: String, state: State<AppState>) -> Result<(), String> {
    let mut engine = state.search_engine.write().map_err(|e| e.to_string())?;
    engine.index_file(&file_id, &content);
    Ok(())
}

#[tauri::command]
fn get_indexed_count(state: State<AppState>) -> Result<usize, String> {
    let engine = state.search_engine.read().map_err(|e| e.to_string())?;
    Ok(engine.indexed_count())
}

// ── Classification Commands ──

#[tauri::command]
async fn classify_files(app: tauri::AppHandle) -> Result<ClassificationResult, String> {
    // Cache-first: if background task already computed, return immediately
    if let Ok(cache) = app.state::<AppState>().classification_cache.lock() {
        if let Some(result) = cache.clone() {
            return Ok(result);
        }
    }

    // Try to rebuild from DB (files already classified in a previous session)
    let app_for_db = app.clone();
    let from_db = tokio::task::spawn_blocking(move || -> Option<ClassificationResult> {
        let state = app_for_db.state::<AppState>();
        let guard = state.store.lock().ok()?;
        let db = guard.as_ref()?;

        let rows = db.get_category_aggregates().ok()?;
        if rows.is_empty() {
            return None; // No existing data, need full classification
        }

        let categories: Vec<CategoryInfo> = rows.into_iter()
            .map(|(name, count, total_size)| CategoryInfo { name: name.clone(), label: name, count, total_size })
            .collect();

        let mut tag_counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        if let Ok(all_tags) = db.get_all_tags() {
            for tags_str in all_tags {
                for tag in tags_str.split(',') {
                    let t = tag.trim();
                    if !t.is_empty() {
                        *tag_counts.entry(t.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut tags: Vec<TagInfo> = tag_counts.into_iter()
            .map(|(name, count)| TagInfo { name, count })
            .collect();
        tags.sort_by(|a, b| b.count.cmp(&a.count));
        tags.truncate(50);

        let result = ClassificationResult { categories, tags };
        if let Ok(mut cache) = state.classification_cache.lock() {
            *cache = Some(result.clone());
        }
        Some(result)
    }).await.unwrap_or(None);

    if let Some(result) = from_db {
        return Ok(result);
    }

    // Otherwise compute synchronously (with progress events)
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let _ = app_clone.emit("classify-progress", serde_json::json!({
            "status": "classifying", "current": 0, "total": 0
        }));
        let result = (|| -> Result<ClassificationResult, String> {
            let state = app_clone.state::<AppState>();
            let files = {
                let guard = state.store.lock().map_err(|e| e.to_string())?;
                guard.as_ref().ok_or_else(|| "No files indexed. Run scan first.".to_string())?
                    .get_all_files().map_err(|e| e.to_string())?
            };
            let (result, cat_map, tag_map) = {
                let engine = state.search_engine.read().map_err(|e| e.to_string())?;
                classifier::classify_files_batch(&files, engine.embedding_engine())
            };
            let _ = app_clone.emit("classify-progress", serde_json::json!({
                "status": "saving", "current": 0, "total": 0
            }));
            if !cat_map.is_empty() || !tag_map.is_empty() {
                if let Some(db) = state.store.lock().map_err(|e| e.to_string())?.as_ref() {
                    for (id, cat) in &cat_map { let _ = db.update_category(id, cat); }
                    for (id, tags) in &tag_map { if !tags.is_empty() { let _ = db.update_tags(id, tags); } }
                }
            }
            Ok(result)
        })();
        match &result {
            Ok(r) => {
                let _ = app_clone.emit("classify-progress", serde_json::json!({
                    "status": "done", "current": 1, "total": 1
                }));
                if let Ok(mut cache) = app_clone.state::<AppState>().classification_cache.lock() {
                    *cache = Some(r.clone());
                }
            }
            Err(e) => {
                log::error!("Classification failed: {}", e);
                let _ = app_clone.emit("classify-progress", serde_json::json!({
                    "status": "error", "error": e
                }));
            }
        }
        result
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

#[tauri::command]
fn get_files_by_category(category_name: String, state: State<AppState>) -> Result<Vec<FileEntry>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_files_by_category(&category_name),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_files_by_category(&category_name)
            } else { Ok(Vec::new()) }
        }
    }
}

#[tauri::command]
fn get_files_by_tag(tag_name: String, state: State<AppState>) -> Result<Vec<FileEntry>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_files_by_tag(&tag_name),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_files_by_tag(&tag_name)
            } else { Ok(Vec::new()) }
        }
    }
}

// ── Custom User Tags Commands ──

#[tauri::command]
fn upsert_user_tag(name: String, state: State<AppState>) -> Result<(), String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.upsert_user_tag(&name),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.upsert_user_tag(&name)
            } else { Err("数据库未初始化".to_string()) }
        }
    }
}

#[tauri::command]
fn get_user_tags(filter: String, limit: u32, state: State<AppState>) -> Result<Vec<(String, u32)>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_user_tags(&filter, limit),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_user_tags(&filter, limit)
            } else { Ok(Vec::new()) }
        }
    }
}

#[tauri::command]
fn get_top_user_tags(limit: u32, state: State<AppState>) -> Result<Vec<(String, u32)>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_top_user_tags(limit),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_top_user_tags(limit)
            } else { Ok(Vec::new()) }
        }
    }
}

#[tauri::command]
fn set_file_custom_tags(file_id: String, tags: Vec<String>, state: State<AppState>) -> Result<(), String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    db.set_file_custom_tags(&file_id, &tags)?;
    // Also record each tag in user history
    for tag in &tags {
        if !tag.trim().is_empty() {
            let _ = db.upsert_user_tag(tag.trim());
        }
    }
    // Clean up orphan tags (tags that no longer belong to any file)
    let _ = db.cleanup_orphan_tags();
    Ok(())
}

#[tauri::command]
fn get_file_custom_tags(file_id: String, state: State<AppState>) -> Result<Vec<String>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_file_custom_tags(&file_id),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_file_custom_tags(&file_id)
            } else { Ok(Vec::new()) }
        }
    }
}

#[tauri::command]
fn get_files_custom_tags_batch(file_ids: Vec<String>, state: State<AppState>) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_files_custom_tags_batch(&file_ids),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_files_custom_tags_batch(&file_ids)
            } else { Ok(std::collections::HashMap::new()) }
        }
    }
}

#[tauri::command]
fn get_all_tags_with_counts(state: State<AppState>) -> Result<Vec<(String, u32)>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_all_tags_with_counts(),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_all_tags_with_counts()
            } else { Ok(Vec::new()) }
        }
    }
}

#[tauri::command]
fn get_files_by_custom_tag(tag: String, state: State<AppState>) -> Result<Vec<FileEntry>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.get_files_by_custom_tag(&tag),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.get_files_by_custom_tag(&tag)
            } else { Ok(Vec::new()) }
        }
    }
}

#[tauri::command]
fn cleanup_orphan_tags(state: State<AppState>) -> Result<u32, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.cleanup_orphan_tags(),
        None => {
            let data_dir = store::get_app_data_dir()?;
            if data_dir.join("metadata.db").exists() {
                MetadataStore::open(&data_dir)?.cleanup_orphan_tags()
            } else { Ok(0) }
        }
    }
}

// ── Chat Commands ──

#[tauri::command]
fn create_chat_session(state: State<AppState>, title: Option<String>) -> Result<ChatSession, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let title = title.unwrap_or_else(|| "新对话".to_string());
    db.create_chat_session(&id, &title)?;
    let session = db.get_chat_session(&id)?.ok_or("创建失败")?;
    Ok(ChatSession {
        id: session.0,
        title: session.1,
        created_at: session.2,
        updated_at: session.3,
        message_count: session.4,
    })
}

#[tauri::command]
fn list_chat_sessions(state: State<AppState>) -> Result<Vec<ChatSession>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    let rows = db.list_chat_sessions()?;
    Ok(rows.into_iter().map(|(id, title, created_at, updated_at, message_count)| {
        ChatSession { id, title, created_at, updated_at, message_count }
    }).collect())
}

#[tauri::command]
fn get_chat_messages(state: State<AppState>, session_id: String) -> Result<Vec<ChatMessage>, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    let rows = db.get_session_messages(&session_id)?;
    Ok(rows.into_iter().map(|(id, sid, role, content, file_refs_json, created_at)| {
        let file_refs = file_refs_json.as_deref()
            .and_then(|json| serde_json::from_str::<Vec<chat::FileRef>>(json).ok());
        ChatMessage { id, session_id: sid, role, content, file_refs, created_at }
    }).collect())
}

#[tauri::command]
fn delete_chat_session(state: State<AppState>, session_id: String) -> Result<(), String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    db.delete_chat_session(&session_id)
}

#[tauri::command]
fn rename_chat_session(state: State<AppState>, session_id: String, title: String) -> Result<(), String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    db.update_chat_session_title(&session_id, &title)
}

#[tauri::command]
async fn chat_send(
    app: tauri::AppHandle,
    session_id: String,
    message: String,
    file_ids: Option<Vec<String>>,
) -> Result<(), String> {
    // ── Save user message + compute file refs (if any) ──
    let app_for_save = app.clone();
    let msg_for_save = message.clone();
    let sid_for_save = session_id.clone();
    let fid_for_save = file_ids.clone();
    let user_save_result = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let state = app_for_save.state::<AppState>();
        let store_lock = state.store.lock().map_err(|e| e.to_string())?;
        let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;

        // Compute file refs for attached files
        let user_file_refs_json: Option<String> = if let Some(ref ids) = fid_for_save {
            if !ids.is_empty() {
                let files = db.get_files_by_ids(ids).unwrap_or_default();
                let refs: Vec<chat::FileRef> = files.iter().map(|f| {
                    let content = db.get_content_text(&f.id).ok().flatten().unwrap_or_default();
                    chat::FileRef {
                        file_id: f.id.clone(),
                        file_name: f.name.clone(),
                        file_path: f.path.clone(),
                        snippet: content.chars().take(300).collect(),
                    }
                }).collect();
                serde_json::to_string(&refs).ok()
            } else { None }
        } else { None };

        let msg_id = uuid::Uuid::new_v4().to_string();
        db.insert_chat_message(&msg_id, &sid_for_save, "user", &msg_for_save, user_file_refs_json.as_deref())?;
        Ok(user_file_refs_json)
    }).await.map_err(|e| format!("Task panicked: {}", e))??;

    // Build file context string for LLM prompt (from attached files)
    let file_context: Option<String> = user_save_result.as_ref().and_then(|json| {
        serde_json::from_str::<Vec<chat::FileRef>>(json).ok().map(|refs| {
            let ctx: String = refs.iter().map(|r| {
                format!("- {} (路径: {})\n  内容预览: {}", r.file_name, r.file_path, r.snippet)
            }).collect::<Vec<_>>().join("\n");
            format!(
                "{}\n\n请分析以上附加文件，你可以给出以下建议：\n\
                1. 文件是否重复或过大需要清理（建议去整理建议页面）\n\
                2. 是否包含隐私信息需要保护（建议使用安全空间加密）\n\
                3. 文件类型是什么，建议归到哪个分类\n\
                4. 是否需要重命名、移动或做其他操作",
                ctx
            )
        })
    });

    // ── Generate assistant response ──
    let app_for_gen = app.clone();
    let sid = session_id.clone();
    let msg = message.clone();
    let fc = file_context.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(String, Option<String>), String> {
        let state = app_for_gen.state::<AppState>();

        // Detect intent
        let intent = chat::detect_intent(&msg);

        // Build RAG context if search intent
        let (rag_context, assistant_file_refs) = match intent {
            ChatIntent::SearchFiles | ChatIntent::SummarizeFile => {
                let search_engine = state.search_engine.read().map_err(|e| e.to_string())?;
                let store_lock = state.store.lock().map_err(|e| e.to_string())?;
                let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
                let bilingual = false;
                match chat::search_to_rag_context(&search_engine, db, &msg, 8, bilingual) {
                    Ok((ctx, refs)) => (Some(ctx), Some(refs)),
                    Err(_) => (None, None),
                }
            }
            _ => (None, None),
        };

        // Serialize assistant file refs for DB
        let assistant_file_refs_json = assistant_file_refs
            .as_ref()
            .and_then(|refs| serde_json::to_string(refs).ok());

        // Load history
        let store_lock = state.store.lock().map_err(|e| e.to_string())?;
        let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
        let history_rows = db.get_session_messages(&sid)?;
        let history: Vec<ChatMessage> = history_rows.into_iter()
            .map(|(id, sid, role, content, file_refs_json, created_at)| {
                let file_refs = file_refs_json.as_deref()
                    .and_then(|json| serde_json::from_str::<Vec<chat::FileRef>>(json).ok());
                ChatMessage { id, session_id: sid, role, content, file_refs, created_at }
            })
            .collect();
        drop(store_lock);

        // Build prompt with both file context and RAG context
        let prompt = chat::build_chat_prompt(&history, &msg, rag_context.as_deref(), fc.as_deref());

        // Check if LLM is loaded
        let llm_loaded = {
            let llm = state.llm_engine.lock().map_err(|e| e.to_string())?;
            llm.is_loaded()
        };

        if !llm_loaded {
            return Err("LLM模型未加载，请先下载模型文件".to_string());
        }

        // Reset cancellation flag
        state.chat_cancelled.store(false, Ordering::Relaxed);

        // Generate with streaming
        let mut llm = state.llm_engine.lock().map_err(|e| e.to_string())?;
        let config = GenerateConfig::default();
        let full_response = llm.generate_streaming(&prompt, &config, Some(&state.chat_cancelled), &mut |token: String| {
            let _ = app_for_gen.emit("chat-token", ChatTokenEvent {
                session_id: sid.clone(),
                token,
                done: false,
            });
        })?;

        Ok((full_response, assistant_file_refs_json))
    }).await.map_err(|e| format!("Task panicked: {}", e))?;

    match result {
        Ok((response, assistant_file_refs_json)) => {
            // Save assistant message with file_refs
            let app_for_save2 = app.clone();
            let sid2 = session_id.clone();
            let refs_json = assistant_file_refs_json.clone();
            tokio::task::spawn_blocking(move || -> Result<(), String> {
                let state = app_for_save2.state::<AppState>();
                let store_lock = state.store.lock().map_err(|e| e.to_string())?;
                let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
                let msg_id = uuid::Uuid::new_v4().to_string();
                db.insert_chat_message(&msg_id, &sid2, "assistant", &response, refs_json.as_deref())?;
                Ok(())
            }).await.map_err(|e| format!("Task panicked: {}", e))??;

            // ── Auto-title: if session title is still "新对话", generate from user message
            let sid_for_title = session_id.clone();
            let title = generate_session_title(&message);
            let state = app.state::<AppState>();
            if let Ok(guard) = state.store.lock() {
                if let Some(db) = guard.as_ref() {
                    if let Ok(Some(session)) = db.get_chat_session(&sid_for_title) {
                        if session.1 == "新对话" {
                            let _ = db.update_chat_session_title(&sid_for_title, &title);
                        }
                    }
                }
            }

            // Emit done event
            let _ = app.emit("chat-token", ChatTokenEvent {
                session_id,
                token: String::new(),
                done: true,
            });
            Ok(())
        }
        Err(e) => {
            let _ = app.emit("chat-token", ChatTokenEvent {
                session_id,
                token: format!("生成失败: {}", e),
                done: true,
            });
            Err(e)
        }
    }
}

// ── Dedup Commands ──

#[tauri::command]
async fn find_duplicates(app: tauri::AppHandle) -> Result<Vec<DuplicateGroup>, String> {
    // Cache-first: if background task already computed, return immediately
    if let Ok(cache) = app.state::<AppState>().duplicate_cache.lock() {
        if let Some(groups) = cache.clone() {
            return Ok(groups);
        }
    }

    // Try to rebuild from DB hashes (previous session's dedup data)
    let app_for_db = app.clone();
    let from_db: Option<Vec<DuplicateGroup>> = tokio::task::spawn_blocking(move || {
        let state = app_for_db.state::<AppState>();
        let guard = state.store.lock().ok()?;
        let db = guard.as_ref()?;
        let entries = db.get_hash_entries().ok()?;
        if entries.is_empty() {
            return None;
        }
        let mut hash_map: std::collections::HashMap<String, Vec<dedup::DuplicateFile>> = std::collections::HashMap::new();
        for (hash, path, name, size, modified) in entries {
            hash_map.entry(hash).or_default().push(dedup::DuplicateFile { name, path, size, modified });
        }
        let groups: Vec<DuplicateGroup> = hash_map.into_iter()
            .filter(|(_, files)| files.len() >= 2)
            .map(|(hash, files)| {
                let total_wasted = files[0].size * (files.len() as u64 - 1);
                DuplicateGroup {
                    id: hash[..12.min(hash.len())].to_string(),
                    total_wasted,
                    reason: "文件内容完全相同".to_string(),
                    files,
                }
            })
            .collect();
        if groups.is_empty() {
            return None;
        }
        if let Ok(mut cache) = state.duplicate_cache.lock() {
            *cache = Some(groups.clone());
        }
        Some(groups)
    }).await.unwrap_or(None);

    if let Some(groups) = from_db {
        return Ok(groups);
    }

    // Otherwise compute synchronously (with progress events)
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let _ = app_clone.emit("dedup-progress", serde_json::json!({
            "status": "hashing", "current": 0, "total": 0
        }));
        let state = app_clone.state::<AppState>();
        let files = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
            Some(db) => db.get_all_files().unwrap_or_default(),
            None => return Err("No files indexed. Run scan first.".to_string()),
        };
        let root = scanner::get_scan_root()?;
        let groups = dedup::find_duplicates(&files, &root);
        let _ = app_clone.emit("dedup-progress", serde_json::json!({
            "status": "done", "current": 1, "total": 1
        }));
        Ok::<Vec<DuplicateGroup>, String>(groups)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

// ── Model Management Commands ──

#[tauri::command]
fn get_models_status() -> Result<Vec<ModelInfo>, String> {
    ai::model_manager::get_models_status()
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ModelLoadStatus { pub id: String, pub loaded: bool }

#[tauri::command]
fn get_model_load_status(state: State<AppState>) -> Result<Vec<ModelLoadStatus>, String> {
    let search = state.search_engine.read().map_err(|e| e.to_string())?;
    let zh_loaded = search.embedding_engine().is_loaded();
    let en_loaded = search.en_embedding_engine()
        .map(|e| e.is_loaded())
        .unwrap_or(false);
    let llm_loaded = state.llm_engine.lock().map_err(|e| e.to_string())?.is_loaded();
    Ok(vec![
        ModelLoadStatus { id: "bge-small-zh".to_string(), loaded: zh_loaded },
        ModelLoadStatus { id: "bge-base-zh".to_string(), loaded: zh_loaded },
        ModelLoadStatus { id: "bge-base-en".to_string(), loaded: en_loaded },
        ModelLoadStatus { id: "qwen2.5-0.5b".to_string(), loaded: llm_loaded },
        ModelLoadStatus { id: "qwen2.5-1.5b".to_string(), loaded: llm_loaded },
        ModelLoadStatus { id: "qwen2.5-7b".to_string(), loaded: llm_loaded },
    ])
}

#[tauri::command]
async fn download_model_file(app: tauri::AppHandle, model_id: String) -> Result<String, String> {
    let app_for_cb = app.clone();
    let app_for_post = app.clone();
    let model_id_cb = model_id.clone();
    let path = tokio::task::spawn_blocking(move || {
        let cb = move |p: ai::model_manager::DownloadProgress| {
            let _ = app_for_cb.emit("download-progress", &p);
        };
        ai::model_manager::download_model(&model_id_cb, cb)
    }).await.map_err(|e| format!("Task failed: {}", e))??;
    let path_clone = path.clone();
    tokio::task::spawn_blocking(move || {
        let state = app_for_post.state::<AppState>();
        match model_id.as_str() {
            "bge-small-zh" | "bge-base-zh" => {
                if let Ok(mut engine) = state.search_engine.write() {
                    if let Err(e) = engine.load_embedding_model(&path_clone) {
                        log::error!("Failed to load embedding: {}", e);
                    }
                }
            }
            "bge-base-en" => {
                if let Ok(mut engine) = state.search_engine.write() {
                    if let Err(e) = engine.load_en_model(&path_clone) {
                        log::error!("Failed to load BGE-base-en: {}", e);
                    }
                }
            }
            "qwen2.5-0.5b" | "qwen2.5-1.5b" | "qwen2.5-7b" => {
                if let Ok(mut engine) = state.llm_engine.lock() {
                    if let Err(e) = engine.load(&path_clone) {
                        log::error!("Failed to load LLM: {}", e);
                    }
                }
            }
            _ => {}
        }
    }).await.map_err(|e| format!("Task failed: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

// ── Vault Commands ──

#[tauri::command]
fn vault_configure(password: String) -> Result<(), String> {
    let vault_dir = vault::get_vault_dir()?;
    vault::configure_vault(&vault_dir, &password)
}

#[tauri::command]
fn vault_unlock(password: String) -> Result<bool, String> {
    let vault_dir = vault::get_vault_dir()?;
    match vault::unlock_vault(&vault_dir, &password) { Ok(_) => Ok(true), Err(e) => Err(e) }
}

#[tauri::command]
fn vault_is_configured() -> Result<bool, String> {
    let vault_dir = vault::get_vault_dir()?;
    Ok(vault::is_vault_configured(&vault_dir))
}

#[tauri::command]
fn vault_add_file(file_path: String, password: String) -> Result<vault::VaultFile, String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;
    let root = scanner::get_scan_root()?;
    let full_path = root.join(&file_path);
    let file_entry = vault::encrypt_file(&full_path, &vault_dir, &key)?;
    vault_update_index(&vault_dir, &key, &file_entry)?;
    Ok(file_entry)
}

#[tauri::command]
fn vault_add_external_file(path: String, password: String) -> Result<vault::VaultFile, String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;
    let full_path = std::path::PathBuf::from(&path);
    if !full_path.exists() {
        return Err(format!("File not found: {}", path));
    }
    let file_entry = vault::encrypt_file(&full_path, &vault_dir, &key)?;
    vault_update_index(&vault_dir, &key, &file_entry)?;
    Ok(file_entry)
}

#[tauri::command]
fn vault_open_file(encrypted_name: String, original_name: String, password: String) -> Result<(), String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;
    let decrypted = vault::decrypt_file(&encrypted_name, &vault_dir, &key)?;
    let temp_dir = std::env::temp_dir().join("semantic-drive-vault");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
    let temp_path = temp_dir.join(&original_name);
    std::fs::write(&temp_path, &decrypted).map_err(|e| format!("Cannot write temp: {}", e))?;
    open::that(&temp_path).map_err(|e| format!("Failed: {}", e))
}

#[tauri::command]
fn vault_delete_file(encrypted_name: String, password: String) -> Result<(), String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;
    vault::remove_vault_file(&encrypted_name, &vault_dir)?;
    // Update index: remove the entry
    let index_path = vault_dir.join("files").join("index.json");
    if index_path.exists() {
        let encrypted = std::fs::read(&index_path).map_err(|e| format!("Cannot read index: {}", e))?;
        let decrypted = vault::decrypt_data(&encrypted, &key)?;
        let mut files: Vec<vault::VaultFile> = serde_json::from_slice(&decrypted).map_err(|e| format!("Parse error: {}", e))?;
        files.retain(|f| f.encrypted_name != encrypted_name);
        let updated = serde_json::to_vec(&files).map_err(|e| format!("Serialize error: {}", e))?;
        let re_encrypted = vault::encrypt_data(&updated, &key)?;
        std::fs::write(&index_path, re_encrypted).map_err(|e| format!("Write error: {}", e))?;
    }
    Ok(())
}

fn vault_update_index(vault_dir: &std::path::Path, key: &[u8; 32], file_entry: &vault::VaultFile) -> Result<(), String> {
    let index_path = vault_dir.join("files").join("index.json");
    let mut files: Vec<vault::VaultFile> = if index_path.exists() {
        let encrypted = std::fs::read(&index_path).map_err(|e| format!("Cannot read index: {}", e))?;
        vault::decrypt_data(&encrypted, key)
            .and_then(|dec| serde_json::from_slice(&dec).map_err(|e| format!("Parse error: {}", e)))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    files.push(file_entry.clone());
    let updated = serde_json::to_vec(&files).map_err(|e| format!("Serialize error: {}", e))?;
    let re_encrypted = vault::encrypt_data(&updated, key)?;
    std::fs::write(&index_path, re_encrypted).map_err(|e| format!("Write error: {}", e))
}

#[tauri::command]
fn vault_list_files(password: String) -> Result<Vec<vault::VaultFile>, String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;
    let index_path = vault_dir.join("files").join("index.json");
    if !index_path.exists() { return Ok(Vec::new()); }
    let encrypted = std::fs::read(&index_path).map_err(|e| format!("Cannot read: {}", e))?;
    let decrypted = vault::decrypt_data(&encrypted, &key)?;
    serde_json::from_slice(&decrypted).map_err(|e| format!("Cannot parse: {}", e))
}

#[tauri::command]
fn vault_remove_file(encrypted_name: String) -> Result<(), String> {
    let vault_dir = vault::get_vault_dir()?;
    vault::remove_vault_file(&encrypted_name, &vault_dir)
}

/// Generate a concise session title from the user's message.
/// Uses a simple heuristic: strip stop words and take first ~20 meaningful chars.
fn generate_session_title(message: &str) -> String {
    let stop_words: &[&str] = &["的", "了", "在", "是", "我", "有", "和", "就", "不", "人", "都",
        "一", "个", "上", "也", "很", "到", "说", "要", "去", "你",
        "会", "着", "没有", "看", "好", "自己", "这", "他", "她", "它",
        "们", "那", "什么", "怎么", "如何", "为什么", "请问", "帮我",
        "找", "搜索", "查", "查找", "寻找", "有没有", "给"];
    let mut cleaned: String = message.chars().filter(|c| !c.is_whitespace()).collect();
    for word in stop_words {
        if cleaned.starts_with(word) {
            cleaned = cleaned[word.len()..].to_string();
            break;
        }
    }
    let title: String = cleaned.chars().take(20).collect();
    let title = title.trim_end_matches(|c: char| c.is_ascii_punctuation() || "。，！？；：、".contains(c));
    if title.is_empty() { "新对话".to_string() } else { title.to_string() }
}

#[tauri::command]
fn stop_chat(state: State<AppState>) -> Result<(), String> {
    state.chat_cancelled.store(true, Ordering::Relaxed);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            store: Mutex::new(None),
            device_root: Mutex::new(None),
            search_engine: RwLock::new(SearchEngine::new()),
            llm_engine: Mutex::new(LlmEngine::new()),
            scan_progress: Mutex::new(None),
            classification_cache: Mutex::new(None),
            duplicate_cache: Mutex::new(None),
            search_cancelled: Mutex::new(false),
            chat_cancelled: AtomicBool::new(false),
        })
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build(),
                )?;
            }
            app.handle().plugin(tauri_plugin_dialog::init())?;
            #[cfg(not(mobile))]
            {
                let state = app.state::<AppState>();
                // Try bge-base-zh first, fall back to bge-small-zh
                if let Ok(Some(path)) = ai::model_manager::get_model_path("bge-base-zh") {
                    if let Ok(mut engine) = state.search_engine.write() {
                        if let Err(e) = engine.load_embedding_model(&path) {
                            log::warn!("BGE-base load failed, trying small: {}", e);
                        }
                    }
                }
                if !state.search_engine.read().map(|e| e.embedding_engine().is_loaded()).unwrap_or(false) {
                    if let Ok(Some(path)) = ai::model_manager::get_model_path("bge-small-zh") {
                        if let Ok(mut engine) = state.search_engine.write() {
                            if let Err(e) = engine.load_embedding_model(&path) {
                                log::warn!("BGE-small load failed: {}", e);
                            }
                        }
                    }
                }
                // Try BGE-base-en for English embedding model
                if let Ok(Some(path)) = ai::model_manager::get_model_path("bge-base-en") {
                    if let Ok(mut engine) = state.search_engine.write() {
                        if let Err(e) = engine.load_en_model(&path) {
                            log::warn!("BGE-base-en load failed: {}", e);
                        }
                    }
                }

                // Try qwen2.5-7b first, fall back to 1.5b, then 0.5b
                if let Ok(Some(path)) = ai::model_manager::get_model_path("qwen2.5-7b") {
                    if let Ok(mut engine) = state.llm_engine.lock() {
                        if let Err(e) = engine.load(&path) {
                            log::warn!("Qwen7B load failed, trying 1.5B: {}", e);
                        }
                    }
                }
                if !state.llm_engine.lock().map(|e| e.is_loaded()).unwrap_or(false) {
                    if let Ok(Some(path)) = ai::model_manager::get_model_path("qwen2.5-1.5b") {
                        if let Ok(mut engine) = state.llm_engine.lock() {
                            if let Err(e) = engine.load(&path) {
                                log::warn!("Qwen1.5B load failed, trying 0.5B: {}", e);
                            }
                        }
                    }
                }
                if !state.llm_engine.lock().map(|e| e.is_loaded()).unwrap_or(false) {
                    if let Ok(Some(path)) = ai::model_manager::get_model_path("qwen2.5-0.5b") {
                        if let Ok(mut engine) = state.llm_engine.lock() {
                            if let Err(e) = engine.load(&path) {
                                log::warn!("Qwen0.5B load failed: {}", e);
                            }
                        }
                    }
                }
                // Open existing database so we don't require re-scan on restart
                if let Ok(root) = scanner::get_device_root() {
                    let data_dir = root.join(".semanticdrive");
                    if data_dir.join("metadata.db").exists() {
                        if let Ok(db) = MetadataStore::open(&data_dir) {
                            if let Ok(mut store_lock) = state.store.lock() {
                                *store_lock = Some(db);
                            }
                        }
                    }
                    if let Ok(scan_root) = scanner::get_scan_root() {
                        if let Ok(mut root_lock) = state.device_root.lock() {
                            *root_lock = Some(scan_root.to_string_lossy().to_string());
                        }
                    }
                }
                // Start filesystem watcher for auto-detecting changes
                if let Ok(scan_root) = scanner::get_scan_root() {
                    let app_watcher = app.handle().clone();
                    if let Ok(watcher) = scanner::start_file_watcher(scan_root, move |event| {
                        use notify::EventKind;
                        let path_str = event.paths.first()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        match event.kind {
                            EventKind::Create(_) => log::info!("[watcher] created: {}", path_str),
                            EventKind::Modify(_) => log::info!("[watcher] modified: {}", path_str),
                            EventKind::Remove(_) => log::info!("[watcher] removed: {}", path_str),
                            _ => {}
                        }
                        // Emit event to frontend so it can refresh if desired
                        let _ = app_watcher.emit("files-changed", serde_json::json!({
                            "kind": format!("{:?}", event.kind),
                            "path": path_str,
                        }));
                    }) {
                        // Leak watcher to keep it alive for the entire app lifetime
                        Box::leak(Box::new(watcher));
                        log::info!("[watcher] started");
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_device_root, scan_files, get_files, get_file_count, get_scan_progress, compute_file_hash,
            search_files, cancel_search, index_file_content, get_indexed_count,
            classify_files, find_duplicates,
            get_models_status, get_model_load_status, download_model_file,
            open_file_location, open_file, open_folder,
            get_files_by_category, get_files_by_tag,
            upsert_user_tag, get_user_tags, get_top_user_tags, set_file_custom_tags, get_file_custom_tags, get_files_custom_tags_batch,
            get_all_tags_with_counts, get_files_by_custom_tag, cleanup_orphan_tags,
            create_chat_session, list_chat_sessions, get_chat_messages, delete_chat_session, rename_chat_session, chat_send, stop_chat,
            get_directory, rename_file, delete_file, move_file, copy_file, import_file,
            vault_configure, vault_unlock, vault_is_configured,
            vault_add_file, vault_add_external_file, vault_open_file, vault_delete_file,
            vault_list_files, vault_remove_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
