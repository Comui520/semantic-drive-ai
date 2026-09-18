mod ai;
mod chat;
mod classifier;
mod dedup;
mod scanner;
mod store;
mod vault;
mod watcher;

use ai::api_client;
use ai::llm::{LlmEngine, ParsedQuery, GenerateConfig};
use ai::search::{SearchEngine, SearchResult};
use classifier::{CategoryInfo, ClassificationResult, TagInfo};
use dedup::DuplicateGroup;
use scanner::FileEntry;
use store::config_store::{AppConfig, ConfigStore};
use store::MetadataStore;
use chat::{ChatSession, ChatMessage, ChatTokenEvent, ChatIntent, FileAction, ChatActionsEvent, parse_actions, strip_action_markers, tool_calls_to_actions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::RwLock;
use std::collections::VecDeque;
use tauri::{Emitter, Manager, State};

pub(crate) struct AppState {
    pub(crate) store: Mutex<Option<MetadataStore>>,
    device_root: Mutex<Option<String>>,
    pub(crate) search_engine: RwLock<SearchEngine>,
    llm_engine: Mutex<LlmEngine>,
    scan_progress: Mutex<Option<scanner::ScanProgress>>,
    classification_cache: Mutex<Option<ClassificationResult>>,
    duplicate_cache: Mutex<Option<Vec<DuplicateGroup>>>,
    search_cancelled: Mutex<bool>,
    chat_cancelled: AtomicBool,
    config_store: Mutex<ConfigStore>,
    vault_key: Mutex<Option<[u8; 32]>>,
    task_queue: Mutex<VecDeque<String>>,
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
    let app_root = scanner::get_device_root()
        .map_err(|e| format!("Cannot get application root: {}", e))?;

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
    let exclude_cb = app_root.clone();
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
    let data_dir = scan_root.join(".semanticdrive");
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

    // Phase 5a (parallel with Phase 3): Incremental classification
    // Only classify new/changed files — existing files already have categories in DB.
    if !need_indexing.is_empty() {
        let app_5a = app.clone();
        let entries_5a = need_indexing.clone();
        tokio::spawn(async move {
            let _ = app_5a.emit("classify-progress", serde_json::json!({
                "status": "classifying", "current": 0, "total": 0
            }));
            let app_bg = app_5a.clone();
            match tokio::task::spawn_blocking(move || -> Result<(), String> {
                let state = app_bg.state::<AppState>();
                let (_, cat_map, tag_map) = {
                    let engine = state.search_engine.read().map_err(|e| e.to_string())?;
                    classifier::classify_files_batch(&entries_5a, engine.embedding_engine())
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
                Ok(())
            }).await {
                Ok(Ok(_)) => {
                    // Clear cache so classify_files command rebuilds aggregates from DB
                    if let Ok(mut cache) = app_5a.state::<AppState>().classification_cache.lock() {
                        *cache = None;
                    }
                    let _ = app_5a.emit("classify-progress", serde_json::json!({
                        "status": "done", "current": 1, "total": 1
                    }));
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
        let llm = state.llm_engine.lock().map_err(|e| e.to_string())?;

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
    let tags_filter = tags.clone(); // clone before move into second spawn_blocking
    let cloud_embedding_endpoint = {
        let state = app.state::<AppState>();
        let config = state.config_store.lock().map_err(|e| e.to_string())?.get_config();
        let (enabled, endpoint) = chat::resolve_embedding_mode(&config);
        if enabled { Some(endpoint) } else { None }
    };

    let results = tokio::task::spawn_blocking(move || {
        // Check cancellation inside the blocking task too
        if *app_clone.state::<AppState>().search_cancelled.lock().map_err(|e| e.to_string())? {
            return Ok::<Vec<SearchResult>, String>(Vec::new());
        }

        let state = app_clone.state::<AppState>();
        let engine = state.search_engine.read().map_err(|e| e.to_string())?;
        let (files, content_scores) = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
            Some(db) => {
                // Pre-filter at SQL level to reduce vector scoring workload
                let file_type_refs: Option<Vec<String>> = if file_types.is_empty() { None } else { Some(file_types.clone()) };
                let time_start = time_range.as_ref().map(|(s, _)| s.as_str());
                let time_end = time_range.as_ref().map(|(_, e)| e.as_str());
                let tag_refs: Option<Vec<String>> = if tags_filter.is_empty() { None } else { Some(tags_filter.clone()) };
                let files = db.get_all_files_filtered(
                    file_type_refs.as_deref(),
                    time_start,
                    time_end,
                    tag_refs.as_deref(),
                    None, // no global limit — let scoring decide top-K
                ).unwrap_or_default();
                let scores = db.search_content_fts5(&search_query_clone).unwrap_or_default();
                (files, scores)
            }
            None => (Vec::new(), Vec::new()),
        };
        let content_score_map: std::collections::HashMap<String, f32> = content_scores.into_iter().collect();

        // Do not mix cloud query vectors with fallback-index vectors. A dedicated
        // cloud re-index job will populate a compatible vector space; until then
        // both query and documents use the deterministic local fallback.
        let query_embedding: Option<Vec<f32>> = None;

        let mut results: Vec<SearchResult> = if has_keywords || !has_time {
            engine.search_with_embedding(&search_query_clone, query_embedding.as_ref(), &files, max_results_clamped * 2, use_en, Some(&content_score_map))
        } else {
            files.iter().map(|f| SearchResult {
                file_id: f.id.clone(), file_name: f.name.clone(), file_path: f.path.clone(),
                score: 0.5, match_type: "时间浏览".to_string(), snippet: String::new(),
                file_size: f.size, modified: f.modified.clone(),
            }).collect()
        };
        // Prefer cloud vectors when a compatible batch index exists. The local
        // fallback remains in place so search is still useful offline.
        if let Some(endpoint) = cloud_embedding_endpoint.as_ref() {
            if let Ok(query_vectors) = api_client::call_embedding_api(endpoint, &[search_query_clone.clone()]) {
                if let Some(query_vector) = query_vectors.first() {
                    if let Ok(guard) = state.store.lock() {
                        if let Some(db) = guard.as_ref() {
                            if let Ok(cloud_vectors) = db.get_cloud_embeddings(&endpoint.model) {
                                let file_map: std::collections::HashMap<&str, &FileEntry> = files.iter().map(|f| (f.id.as_str(), f)).collect();
                                let mut cloud_scores = std::collections::HashMap::new();
                                for (file_id, vector) in cloud_vectors {
                                    if let Some(file) = file_map.get(file_id.as_str()) {
                                        let dot: f32 = query_vector.iter().zip(vector.iter()).map(|(a, b)| a * b).sum();
                                        let q_norm = query_vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                                        let v_norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                                        if q_norm > 0.0 && v_norm > 0.0 {
                                            cloud_scores.insert(file_id, ((dot / (q_norm * v_norm)) + 1.0) / 2.0);
                                        }
                                        let _ = file;
                                    }
                                }
                                let existing_ids: std::collections::HashSet<String> = results.iter().map(|result| result.file_id.clone()).collect();
                                for result in &mut results {
                                    if let Some(score) = cloud_scores.get(&result.file_id) {
                                        result.score = (result.score * 0.35 + score * 0.65).min(1.0);
                                        result.match_type = "云端语义匹配".to_string();
                                    }
                                }
                                // The local in-memory index is intentionally not
                                // persisted. Rehydrate cloud-only hits after restart.
                                for file in &files {
                                    if existing_ids.contains(&file.id) { continue; }
                                    if let Some(score) = cloud_scores.get(&file.id) {
                                        if *score >= 0.35 {
                                            results.push(SearchResult {
                                                file_id: file.id.clone(), file_name: file.name.clone(), file_path: file.path.clone(),
                                                score: *score, match_type: "云端语义匹配".to_string(), snippet: String::new(),
                                                file_size: file.size, modified: file.modified.clone(),
                                            });
                                        }
                                    }
                                }
                                results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
                            }
                        }
                    }
                }
            }
        }
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
fn create_directory(dir_path: String) -> Result<(), String> {
    let root = scanner::get_scan_root()?;
    let full = root.join(&dir_path);
    std::fs::create_dir_all(&full).map_err(|e| format!("Create directory failed: {}", e))
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
    folder_paths: Option<Vec<String>>,
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
            let path_list: String = refs.iter().map(|r| {
                format!("  {} (ID: {})", r.file_path, r.file_id)
            }).collect::<Vec<_>>().join("\n");
            let ctx: String = refs.iter().map(|r| {
                format!("- {} (ID: {}, 路径: {})\n  内容预览: {}", r.file_name, r.file_id, r.file_path, r.snippet)
            }).collect::<Vec<_>>().join("\n");
            format!(
                "附加文件路径列表（请原样使用这些路径）：\n{}\n\n{}\n\n请分析以上附加文件，你可以给出以下建议：\n\
                1. 文件是否重复或过大需要清理（建议去整理建议页面）\n\
                2. 是否包含隐私信息需要保护（建议使用安全空间加密）\n\
                3. 文件类型是什么，建议归到哪个分类\n\
                4. 是否需要重命名、移动或做其他操作",
                path_list, ctx
            )
        })
    });

    // ── Build folder context (structured, not emoji-dependent) ──
    let folder_context: Option<String> = folder_paths.as_ref().and_then(|paths| {
        if paths.is_empty() { return None; }
        Some(format!(
            "附加文件夹路径（用户指定的操作目标位置）：\n{}\n\n\
             **重要：当用户要求将文件放入上述文件夹时，直接将文件夹路径作为 destination 使用，不要添加额外子目录层级。**",
            paths.iter().map(|p| format!("  📁 {}", p)).collect::<Vec<_>>().join("\n")
        ))
    });

    // Merge file context and folder context
    let combined_context: Option<String> = match (file_context.as_ref(), folder_context) {
        (Some(fc), Some(foc)) => Some(format!("{}\n\n{}", fc, foc)),
        (Some(fc), None) => Some(fc.clone()),
        (None, Some(foc)) => Some(foc),
        (None, None) => None,
    };

    // ── Generate assistant response ──
    let app_for_gen = app.clone();
    let sid = session_id.clone();
    let msg = message.clone();
    let fc = combined_context.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(String, Option<String>), String> {
        let state = app_for_gen.state::<AppState>();

        // Detect intent
        let intent = chat::detect_intent(&msg);

        // Build RAG context if search intent (skip if files already attached)
        let has_attached_files = fc.is_some();
        let (rag_context, assistant_file_refs) = match intent {
            ChatIntent::SearchFiles | ChatIntent::SummarizeFile if !has_attached_files => {
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

        // Reset cancellation flag
        state.chat_cancelled.store(false, Ordering::Relaxed);

        let gen_config = GenerateConfig::default();

        let endpoint = {
            let config_store = state.config_store.lock().map_err(|e| e.to_string())?;
            let config = config_store.get_config();
            let (enabled, endpoint) = chat::resolve_chat_mode(&config);
            if !enabled {
                return Err("尚未配置可用的聊天 API。请在设置中填写 API 地址、模型和密钥后重试。".to_string());
            }
            endpoint
        };

        let messages = chat::build_chat_messages_api(
            &history, &msg, rag_context.as_deref(), fc.as_deref(),
        );
        let tool_defs = chat::action_tools();
        let mut emit_token = |token: String| {
            let _ = app_for_gen.emit("chat-token", ChatTokenEvent {
                session_id: sid.clone(), token, done: false,
            });
        };
        let completion = match api_client::call_chat_completion_streaming_api_with_tools(
            &endpoint, &messages, gen_config.temperature, gen_config.max_tokens,
            &tool_defs, Some(&state.chat_cancelled), &mut emit_token,
        ) {
            Ok(value) => value,
            Err(tool_error) => {
                // A few older OpenAI-compatible gateways reject `tools`. Retry
                // without the extension so API-first mode remains compatible.
                log::warn!("Native tool calling unavailable, retrying legacy chat: {}", tool_error);
                let text = api_client::call_chat_completion_streaming_api(
                    &endpoint, &messages, gen_config.temperature, gen_config.max_tokens,
                    Some(&state.chat_cancelled), &mut emit_token,
                )?;
                api_client::ChatCompletionResult { text, tool_calls: Vec::new() }
            }
        };
        // Native tool calling is normalized to the same action envelope used by
        // older providers. This keeps confirmation, validation, and history in
        // one code path while allowing true JSON-schema tool calls by default.
        let tool_actions = tool_calls_to_actions(&completion.tool_calls);
        let mut full_response = completion.text;
        for action in &tool_actions {
            if let Ok(json) = serde_json::to_string(action) {
                full_response.push_str(&format!("\n[ACTION:{}]", json));
            }
        }
        Ok((full_response, assistant_file_refs_json))
    }).await.map_err(|e| format!("Task panicked: {}", e))?;

    match result {
        Ok((response, assistant_file_refs_json)) => {
            // Reject empty responses — model generated nothing
            if response.trim().is_empty() {
                return Err("模型未生成有效回复，请重试".to_string());
            }

            // ── AI Actions Engine ──
            let actions = parse_actions(&response);
            let clean_text = if actions.is_empty() {
                response.clone()
            } else {
                strip_action_markers(&response)
            };

            if !actions.is_empty() {
                let _ = app.emit("chat-actions", ChatActionsEvent {
                    session_id: session_id.clone(),
                    actions,
                });
            }

            // Save assistant message (with action markers stripped)
            let app_for_save2 = app.clone();
            let sid2 = session_id.clone();
            let refs_json = assistant_file_refs_json.clone();
            let value = clean_text.clone();
            tokio::task::spawn_blocking(move || -> Result<(), String> {
                let state = app_for_save2.state::<AppState>();
                let store_lock = state.store.lock().map_err(|e| e.to_string())?;
                let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
                let msg_id = uuid::Uuid::new_v4().to_string();
                db.insert_chat_message(&msg_id, &sid2, "assistant", &value, refs_json.as_deref())?;
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

// ── AI Action Execution ──

/// Sanitize file paths from LLM output — the model sometimes hallucinates
/// leading slashes, URL encoding, or stray whitespace in paths.
fn sanitize_action_path(path: &str) -> String {
    let path = path.trim();
    let path = path.trim_start_matches('/');
    #[cfg(windows)]
    let path = path.trim_start_matches('\\');
    let path = path.replace('\\', "/");
    let path = path.replace("//", "/");
    // Manually decode percent-encoded sequences (LLM sometimes URL-encodes spaces/Chinese)
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }
    // Normalize ".." path segments to prevent path traversal from LLM output.
    // e.g. ../BJUTJava → BJUTJava,  a/../b → a/b
    // Preserve trailing slash to distinguish directory paths.
    let ends_with_slash = result.ends_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in result.split('/') {
        if segment == ".." {
            segments.pop();
        } else if segment == "." || segment.is_empty() {
            continue;
        } else {
            segments.push(segment);
        }
    }
    result = segments.join("/");
    if ends_with_slash {
        result.push('/');
    }
    result
}

/// Try to resolve a path from LLM output to an actual file on disk.
/// 1. Sanitize + join with root
/// 2. If the file doesn't exist, query DB for files in the same directory
///    and find the closest match by character overlap (handles trad→simpl, etc.)
/// Returns (full PathBuf, the resolved relative path string).
fn resolve_action_path(
    llm_path: &str,
    root: &std::path::Path,
    state: &AppState,
) -> Result<(std::path::PathBuf, String), String> {
    let cleaned = sanitize_action_path(llm_path);
    let full = root.join(&cleaned);

    // 1. Exact match
    if full.exists() {
        return Ok((full, cleaned));
    }

    // 2. Fuzzy match against DB files in same directory
    let llm_filename = cleaned.rsplit('/').next().unwrap_or(&cleaned);

    // Determine parent directory — handle paths without a slash (top-level files)
    let parent_dir = if cleaned.contains('/') {
        let idx = cleaned.rfind('/').unwrap();
        &cleaned[..idx]
    } else {
        ""
    };

    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;

    // Get all files whose parent directory matches
    let all_files = db.get_all_files().map_err(|e| e.to_string())?;
    let candidates: Vec<&FileEntry> = all_files.iter()
        .filter(|f| {
            if parent_dir.is_empty() {
                !f.path.contains('/')
            } else {
                f.path.starts_with(parent_dir)
                    && f.path[parent_dir.len()..].starts_with('/')
            }
        })
        .collect();

    if !candidates.is_empty() {
        // Score each candidate by how many characters from the LLM filename
        // appear in the candidate filename in order (character-by-character match).
        let mut best_score: isize = -1;
        let mut best_path: Option<&str> = None;

        for candidate in &candidates {
            let cand_filename = candidate.path.rsplit('/').next().unwrap_or(&candidate.path);

            // Count matching characters in sequence (longest common subsequence style)
            let score = cand_filename.chars()
                .filter(|&c| llm_filename.contains(c))
                .count() as isize;

            // Penalize if the candidate filename is much longer than the LLM version
            let len_diff = (cand_filename.len() as isize - llm_filename.len() as isize).abs();
            let score = score - len_diff;

            if score > best_score {
                best_score = score;
                best_path = Some(&candidate.path);
            }
        }

        if best_score > 0 {
            if let Some(best) = best_path {
                let full = root.join(best);
                if full.exists() {
                    return Ok((full, best.to_string()));
                }
            }
        }
    }

    // 3. Broader search: match by any path segment
    for file in &all_files {
        if file.path.contains(llm_filename.trim()) {
            let full = root.join(&file.path);
            if full.exists() {
                return Ok((full, file.path.clone()));
            }
        }
    }

    Err(format!("文件不存在: {}", cleaned))
}

/// Resolve a file_id to its stored relative path from the database.
fn get_file_path_by_id(state: &AppState, file_id: &str) -> Result<String, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
    let files = db.get_files_by_ids(&[file_id.to_string()])
        .map_err(|e| format!("查询文件失败: {}", e))?;
    files.into_iter()
        .next()
        .map(|f| f.path)
        .ok_or_else(|| format!("文件不存在 (ID: {})", file_id))
}

/// Resolve a database path while enforcing the current scan-root boundary.
fn resolve_indexed_path(state: &AppState, file_id: &str, root: &std::path::Path) -> Result<(std::path::PathBuf, String), String> {
    let relative = get_file_path_by_id(state, file_id)?;
    let relative_path = std::path::Path::new(&relative);
    if relative_path.is_absolute() || relative_path.components().any(|component| matches!(component, std::path::Component::Prefix(_))) {
        return Err("索引中的文件路径无效".to_string());
    }
    let full = root.join(relative_path);
    let canonical_root = std::fs::canonicalize(root).map_err(|e| format!("无法解析扫描目录: {e}"))?;
    let canonical_file = std::fs::canonicalize(&full).map_err(|e| format!("文件不存在: {relative} ({e})"))?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err("文件不在已扫描目录内".to_string());
    }
    Ok((canonical_file, relative))
}

fn validate_new_name(name: &str) -> Result<(), String> {
    let path = std::path::Path::new(name.trim());
    if name.trim().is_empty() || path.file_name().and_then(|n| n.to_str()) != Some(name.trim()) || name.contains(['/', '\\']) {
        return Err("新文件名只能包含文件名，不能包含目录分隔符".to_string());
    }
    Ok(())
}

/// If destination is a directory (ends with `/`, already exists as a dir on disk,
/// or has no file extension), append the source filename to make a complete target path.
fn resolve_destination(dest: &str, src: &std::path::Path, root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let relative = std::path::Path::new(dest);
    if relative.is_absolute() || relative.components().any(|component| matches!(component, std::path::Component::Prefix(_))) {
        return Err("目标路径必须位于已扫描目录内".to_string());
    }
    let dst = root.join(relative);
    if dest.ends_with('/') || dst.is_dir() || !dest.split('/').last().map_or(false, |s| s.contains('.')) {
        let name = src.file_name()
            .ok_or_else(|| "无法获取源文件名".to_string())?;
        Ok(dst.join(name))
    } else {
        Ok(dst)
    }
}

/// Execute a `FileAction` that was requested by the AI assistant.
/// Called from the frontend after user confirmation.
fn execute_file_action_inner(action_json: String, state: &AppState) -> Result<String, String> {
    let action: FileAction = serde_json::from_str(&action_json)
        .map_err(|e| format!("无效的操作: {}", e))?;

    match action {
        // ── Legacy path-based actions (fallback) ──

        FileAction::RenameFile { old_path, new_name } => {
            validate_new_name(&new_name)?;
            let root = scanner::get_scan_root()?;
            let (full_old, old_path) = resolve_action_path(&old_path, &root, &state)?;
            let full_new = full_old.with_file_name(&new_name);
            std::fs::rename(&full_old, &full_new)
                .map_err(|e| format!("重命名失败: {}", e))?;
            Ok(format!("已重命名「{}」→「{}」", old_path, new_name))
        }
        FileAction::DeleteFile { file_path } => {
            let root = scanner::get_scan_root()?;
            let (full, file_path) = resolve_action_path(&file_path, &root, &state)?;
            if full.is_dir() {
                std::fs::remove_dir_all(&full).map_err(|e| format!("删除失败: {}", e))?;
            } else {
                std::fs::remove_file(&full).map_err(|e| format!("删除失败: {}", e))?;
            }
            Ok(format!("已删除: {}", file_path))
        }
        FileAction::MoveFile { source, destination } => {
            let destination = sanitize_action_path(&destination);
            let root = scanner::get_scan_root()?;
            let (src, source) = resolve_action_path(&source, &root, &state)?;
            let dst = resolve_destination(&destination, &src, &root)?;
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }
            std::fs::rename(&src, &dst)
                .map_err(|e| format!("移动失败: {}", e))?;
            let display = dst.strip_prefix(&root).unwrap_or(&dst).to_string_lossy();
            Ok(format!("已移动「{}」→「{}」", source, display))
        }
        FileAction::CopyFile { source, destination } => {
            let destination = sanitize_action_path(&destination);
            let root = scanner::get_scan_root()?;
            let (src, source) = resolve_action_path(&source, &root, &state)?;
            let dst = resolve_destination(&destination, &src, &root)?;
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }
            if src.is_dir() {
                copy_dir_recursive(&src, &dst)?;
            } else {
                std::fs::copy(&src, &dst)
                    .map_err(|e| format!("复制失败: {}", e))?;
            }
            let display = dst.strip_prefix(&root).unwrap_or(&dst).to_string_lossy();
            Ok(format!("已复制「{}」→「{}」", source, display))
        }
        FileAction::ImportFile { source, destination } => {
            let destination = sanitize_action_path(&destination);
            let root = scanner::get_scan_root()?;
            // Sanitize source to prevent path traversal (LLM may hallucinate paths)
            let cleaned_source = sanitize_action_path(&source);
            let src = std::path::PathBuf::from(&cleaned_source);
            if !src.exists() {
                return Err(format!("文件不存在: {}", cleaned_source));
            }
            let dst = root.join(&destination);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("导入失败: {}", e))?;
            Ok(format!("已导入「{}」→「{}」", cleaned_source, destination))
        }
        FileAction::VaultAddFile { file_path, password: _pw } => {
            let vault_dir = vault::get_vault_dir()?;
            let key = {
                let vk = state.vault_key.lock().map_err(|e| e.to_string())?;
                vk.ok_or_else(|| "请先在安全空间页面解锁".to_string())?
            };
            let root = scanner::get_scan_root()?;
            let (full_path, file_path) = resolve_action_path(&file_path, &root, &state)?;
            vault::encrypt_file(&full_path, &vault_dir, &key)
                .map_err(|e| format!("加密失败: {}", e))?;
            Ok(format!("已加密添加到安全空间: {}", file_path))
        }
        FileAction::SetFileTags { file_id, tags } => {
            let store_lock = state.store.lock().map_err(|e| e.to_string())?;
            let db = store_lock.as_ref()
                .ok_or_else(|| "数据库未初始化".to_string())?;
            db.set_file_custom_tags(&file_id, &tags)
                .map_err(|e| format!("设置标签失败: {}", e))?;
            for tag in &tags {
                let _ = db.upsert_user_tag(tag);
            }
            Ok(format!("已设置标签: {}", tags.join(", ")))
        }

        // ── File-ID-based actions (preferred — no path hallucination) ──

        FileAction::MoveFileById { file_id, destination } => {
            let root = scanner::get_scan_root()?;
            let (src, source) = resolve_indexed_path(&state, &file_id, &root)?;
            let destination = sanitize_action_path(&destination);
            let dst = resolve_destination(&destination, &src, &root)?;
            if !src.exists() {
                return Err(format!("文件不存在: {}", source));
            }
            if src == dst {
                let display = dst.strip_prefix(&root).unwrap_or(&dst).to_string_lossy().to_string();
                return Ok(format!("文件已在目标位置: {}", display));
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }
            std::fs::rename(&src, &dst)
                .map_err(|e| format!("移动失败: {}", e))?;
            let display = dst.strip_prefix(&root).unwrap_or(&dst).to_string_lossy().to_string();
            // Update DB so the file remains findable at its new location
            if let Some(name) = dst.file_name() {
                if let Ok(store_lock) = state.store.lock() {
                    if let Some(db) = store_lock.as_ref() {
                        let _ = db.update_file_path(&file_id, &display, &name.to_string_lossy());
                    }
                }
            }
            Ok(format!("已移动「{}」→「{}」", source, display))
        }
        FileAction::CopyFileById { file_id, destination } => {
            let root = scanner::get_scan_root()?;
            let (src, source) = resolve_indexed_path(&state, &file_id, &root)?;
            let destination = sanitize_action_path(&destination);
            let dst = resolve_destination(&destination, &src, &root)?;
            if !src.exists() {
                return Err(format!("文件不存在: {}", source));
            }
            if src == dst {
                let display = dst.strip_prefix(&root).unwrap_or(&dst).to_string_lossy().to_string();
                return Ok(format!("文件已在目标位置: {}", display));
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }
            if src.is_dir() {
                copy_dir_recursive(&src, &dst)?;
            } else {
                std::fs::copy(&src, &dst)
                    .map_err(|e| format!("复制失败: {}", e))?;
            }
            let display = dst.strip_prefix(&root).unwrap_or(&dst).to_string_lossy();
            Ok(format!("已复制「{}」→「{}」", source, display))
        }
        FileAction::DeleteFileById { file_id } => {
            let root = scanner::get_scan_root()?;
            let (full, file_path) = resolve_indexed_path(&state, &file_id, &root)?;
            if !full.exists() {
                return Err(format!("文件不存在: {}", file_path));
            }
            if full.is_dir() {
                std::fs::remove_dir_all(&full).map_err(|e| format!("删除失败: {}", e))?;
            } else {
                std::fs::remove_file(&full).map_err(|e| format!("删除失败: {}", e))?;
            }
            // Remove from DB so it no longer appears in search
            if let Ok(store_lock) = state.store.lock() {
                if let Some(db) = store_lock.as_ref() {
                    let _ = db.remove_file(&file_id);
                }
            }
            Ok(format!("已删除: {}", file_path))
        }
        FileAction::RenameFileById { file_id, new_name } => {
            validate_new_name(&new_name)?;
            let root = scanner::get_scan_root()?;
            let (full_old, old_path) = resolve_indexed_path(&state, &file_id, &root)?;
            if !full_old.exists() {
                return Err(format!("文件不存在: {}", old_path));
            }
            let full_new = full_old.with_file_name(&new_name);
            std::fs::rename(&full_old, &full_new)
                .map_err(|e| format!("重命名失败: {}", e))?;
            // Update DB so the file remains findable at its new name
            if let Ok(store_lock) = state.store.lock() {
                if let Some(db) = store_lock.as_ref() {
                    let new_path = full_new.strip_prefix(&root).unwrap_or(&full_new).to_string_lossy().to_string();
                    let _ = db.update_file_path(&file_id, &new_path, &new_name);
                }
            }
            Ok(format!("已重命名「{}」→「{}」", old_path, new_name))
        }
        FileAction::VaultAddFileById { file_id, password: _pw } => {
            let vault_dir = vault::get_vault_dir()?;
            let key = {
                let vk = state.vault_key.lock().map_err(|e| e.to_string())?;
                vk.ok_or_else(|| "请先在安全空间页面解锁".to_string())?
            };
            let root = scanner::get_scan_root()?;
            let (full_path, relative) = resolve_indexed_path(&state, &file_id, &root)?;
            if !full_path.exists() {
                return Err(format!("文件不存在: {}", relative));
            }
            vault::encrypt_file(&full_path, &vault_dir, &key)
                .map_err(|e| format!("加密失败: {}", e))?;
            Ok(format!("已加密添加到安全空间: {}", relative))
        }
        FileAction::SearchFiles { query } => Ok(format!("搜索: {}", query)),
        FileAction::ClassifyFiles => Ok("已打开文件分类页面".to_string()),
        FileAction::FindDuplicates => Ok("已打开去重检测页面".to_string()),
        FileAction::ImportFileById { file_id, destination } => {
            let root = scanner::get_scan_root()?;
            let (src, source) = resolve_indexed_path(&state, &file_id, &root)?;
            let destination = sanitize_action_path(&destination);
            let dst = resolve_destination(&destination, &src, &root)?;
            if !src.exists() {
                return Err(format!("文件不存在: {}", source));
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败: {}", e))?;
            }
            std::fs::copy(&src, &dst)
                .map_err(|e| format!("导入失败: {}", e))?;
            Ok(format!("已导入「{}」→「{}」", source, destination))
        }
        FileAction::OpenFileById { file_id } => {
            let root = scanner::get_scan_root()?;
            let (full, file_path) = resolve_indexed_path(&state, &file_id, &root)?;
            if !full.exists() {
                return Err(format!("文件不存在: {}", file_path));
            }
            open::that(full).map_err(|e| format!("打开失败: {}", e))?;
            Ok(format!("已打开: {}", file_path))
        }
        FileAction::OpenFileLocationById { file_id } => {
            let root = scanner::get_scan_root()?;
            let (full, file_path) = resolve_indexed_path(&state, &file_id, &root)?;
            if !full.exists() {
                return Err(format!("文件不存在: {}", file_path));
            }
            #[cfg(target_os = "windows")]
            {
                std::process::Command::new("explorer")
                    .arg(format!("/select,{}", full.display()))
                    .spawn()
                    .map_err(|e| format!("打开位置失败: {}", e))?;
            }
            #[cfg(not(target_os = "windows"))]
            {
                let parent = full.parent().ok_or("无父目录")?;
                open::that(parent).map_err(|e| format!("打开位置失败: {}", e))?;
            }
            Ok(format!("已打开位置: {}", file_path))
        }
        FileAction::AddFileTags { file_id, tags } => {
            let store_lock = state.store.lock().map_err(|e| e.to_string())?;
            let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
            // Get existing tags, merge, and set
            let existing = db.get_file_custom_tags(&file_id).unwrap_or_default();
            let mut merged = existing.clone();
            for tag in &tags {
                if !merged.contains(tag) { merged.push(tag.clone()); }
            }
            db.set_file_custom_tags(&file_id, &merged)
                .map_err(|e| format!("添加标签失败: {}", e))?;
            for tag in &tags {
                let _ = db.upsert_user_tag(tag);
            }
            Ok(format!("已添加标签: {}", tags.join("、")))
        }
        FileAction::RemoveFileTags { file_id, tags } => {
            let store_lock = state.store.lock().map_err(|e| e.to_string())?;
            let db = store_lock.as_ref().ok_or_else(|| "数据库未初始化".to_string())?;
            let existing = db.get_file_custom_tags(&file_id).unwrap_or_default();
            let filtered: Vec<String> = existing.into_iter()
                .filter(|t| !tags.contains(t))
                .collect();
            db.set_file_custom_tags(&file_id, &filtered)
                .map_err(|e| format!("移除标签失败: {}", e))?;
            let _ = db.cleanup_orphan_tags();
            Ok(format!("已移除标签: {}", tags.join("、")))
        }
    }
}


fn inverse_action_for(action: &FileAction, state: &AppState) -> Option<FileAction> {
    let db_guard = state.store.lock().ok()?;
    let db = db_guard.as_ref()?;
    match action {
        FileAction::RenameFileById { file_id, .. } => {
            let old_path = db.get_files_by_ids(&[file_id.clone()]).ok()?.into_iter().next()?.path;
            let old_name = old_path.rsplit('/').next()?.to_string();
            Some(FileAction::RenameFileById { file_id: file_id.clone(), new_name: old_name })
        }
        FileAction::MoveFileById { file_id, .. } => {
            let old_path = db.get_files_by_ids(&[file_id.clone()]).ok()?.into_iter().next()?.path;
            Some(FileAction::MoveFileById { file_id: file_id.clone(), destination: old_path })
        }
        FileAction::SetFileTags { file_id, .. } => {
            let tags = db.get_file_custom_tags(file_id).ok()?;
            Some(FileAction::SetFileTags { file_id: file_id.clone(), tags })
        }
        FileAction::AddFileTags { file_id, tags } => Some(FileAction::RemoveFileTags { file_id: file_id.clone(), tags: tags.clone() }),
        FileAction::RemoveFileTags { file_id, tags } => Some(FileAction::AddFileTags { file_id: file_id.clone(), tags: tags.clone() }),
        _ => None,
    }
}

/// Execute a confirmed Agent action and persist an auditable history record.
#[tauri::command]
fn execute_file_action(action_json: String, state: State<AppState>) -> Result<String, String> {
    let action: FileAction = serde_json::from_str(&action_json)
        .map_err(|e| format!("无效的操作: {}", e))?;
    let inverse = inverse_action_for(&action, &state);
    let result = execute_file_action_inner(action_json.clone(), &state)?;
    if let Ok(store_guard) = state.store.lock() {
        if let Some(db) = store_guard.as_ref() {
            let inverse_json = inverse.as_ref().and_then(|value| serde_json::to_string(value).ok());
            let _ = db.record_action_history(&uuid::Uuid::new_v4().to_string(), &action_json, inverse_json.as_deref(), &result);
        }
    }
    Ok(result)
}

#[tauri::command]
fn list_action_history(state: State<AppState>, limit: Option<u32>) -> Result<Vec<store::ActionHistoryEntry>, String> {
    let guard = state.store.lock().map_err(|e| e.to_string())?;
    guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.list_action_history(limit.unwrap_or(50).min(500))
}

#[tauri::command]
fn undo_action(state: State<AppState>, history_id: String) -> Result<String, String> {
    let entry = {
        let guard = state.store.lock().map_err(|e| e.to_string())?;
        guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.get_action_history(&history_id)?
    }.ok_or_else(|| "找不到操作记录".to_string())?;
    if entry.status != "completed" { return Err("该操作已经撤销或不可撤销".to_string()); }
    let inverse_json = entry.inverse_action_json.ok_or_else(|| "该操作暂不支持撤销".to_string())?;
    let result = execute_file_action_inner(inverse_json, &state)?;
    let guard = state.store.lock().map_err(|e| e.to_string())?;
    guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.mark_action_undone(&history_id)?;
    Ok(result)
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


fn spawn_cloud_embedding_rebuild(app: tauri::AppHandle, task_id: String, endpoint: api_client::ApiEndpointConfig) {
    tokio::spawn(async move {
        let app_bg = app.clone();
        let task_id_for_worker = task_id.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let state = app_bg.state::<AppState>();
            if let Ok(mut queue) = state.task_queue.lock() { queue.push_back(task_id_for_worker.clone()); }
            let update = |status: &str, progress: u32, result: Option<&str>, error: Option<&str>| {
                if let Ok(guard) = state.store.lock() {
                    if let Some(db) = guard.as_ref() { let _ = db.update_agent_task(&task_id_for_worker, status, progress, result, error); }
                }
                let _ = app_bg.emit("task-progress", serde_json::json!({"task_id": task_id_for_worker, "status": status, "progress": progress}));
            };
            update("running", 0, None, None);
            let files = {
                let guard = state.store.lock().map_err(|e| e.to_string())?;
                guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.get_all_files()?
            };
            let mut items: Vec<(String, String, Option<String>)> = Vec::new();
            for file in files {
                let content = {
                    let guard = state.store.lock().map_err(|e| e.to_string())?;
                    guard.as_ref().and_then(|db| db.get_content_text(&file.id).ok().flatten())
                };
                if let Some(text) = content.filter(|text| text.trim().len() >= 20) {
                    items.push((file.id, text, file.hash));
                }
            }
            let total = items.len().max(1);
            for (chunk_index, chunk) in items.chunks(16).enumerate() {
                let cancelled = {
                    let guard = state.store.lock().map_err(|e| e.to_string())?;
                    guard.as_ref().and_then(|db| db.get_agent_tasks(500).ok()).map(|tasks| tasks.iter().any(|t| t.id == task_id_for_worker && t.status == "cancelled")).unwrap_or(false)
                };
                if cancelled { update("cancelled", ((chunk_index * 16 * 100) / total) as u32, None, None); return Ok(()); }
                let texts: Vec<String> = chunk.iter().map(|(_, text, _)| text.clone()).collect();
                let vectors = api_client::call_embedding_api(&endpoint, &texts)?;
                for ((file_id, _, hash), vector) in chunk.iter().zip(vectors.iter()) {
                    let guard = state.store.lock().map_err(|e| e.to_string())?;
                    if let Some(db) = guard.as_ref() { db.update_cloud_embedding(file_id, &endpoint.model, hash.as_deref(), vector)?; }
                }
                let progress = (((chunk_index + 1) * 16 * 100) / total).min(100) as u32;
                update("running", progress, None, None);
            }
            update("completed", 100, Some(&format!("已重建 {} 个云端向量", items.len())), None);
            if let Ok(mut queue) = state.task_queue.lock() { queue.retain(|id| id != &task_id_for_worker); }
            Ok(())
        }).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let state = app.state::<AppState>();
                if let Ok(guard) = state.store.lock() { if let Some(db) = guard.as_ref() { let _ = db.update_agent_task(&task_id, "failed", 0, None, Some(&error)); } }
                let _ = app.emit("task-progress", serde_json::json!({"task_id": task_id, "status": "failed", "error": error}));
            }
            Err(error) => {
                let message = format!("任务线程异常: {}", error);
                let state = app.state::<AppState>();
                if let Ok(guard) = state.store.lock() { if let Some(db) = guard.as_ref() { let _ = db.update_agent_task(&task_id, "failed", 0, None, Some(&message)); } };
            }
        }
    });
}

#[tauri::command]
async fn rebuild_cloud_embeddings(app: tauri::AppHandle) -> Result<String, String> {
    let endpoint = {
        let state = app.state::<AppState>();
        let config = state.config_store.lock().map_err(|e| e.to_string())?.get_config();
        let (enabled, endpoint) = chat::resolve_embedding_mode(&config);
        if !enabled { return Err("尚未配置可用的 Embedding API。请先在设置中填写地址、模型和密钥。".to_string()); }
        endpoint
    };
    let task_id = uuid::Uuid::new_v4().to_string();
    {
        let state = app.state::<AppState>();
        let guard = state.store.lock().map_err(|e| e.to_string())?;
        let db = guard.as_ref().ok_or_else(|| "数据库未初始化，请先扫描工作区".to_string())?;
        db.create_agent_task(&task_id, "embedding_rebuild", &serde_json::json!({"model": endpoint.model}).to_string())?;
    }
    spawn_cloud_embedding_rebuild(app, task_id.clone(), endpoint);
    Ok(task_id)
}

#[tauri::command]
fn list_agent_tasks(state: State<AppState>, limit: Option<u32>) -> Result<Vec<store::AgentTask>, String> {
    let guard = state.store.lock().map_err(|e| e.to_string())?;
    guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.get_agent_tasks(limit.unwrap_or(50).min(500))
}

#[tauri::command]
fn cancel_agent_task(state: State<AppState>, task_id: String) -> Result<(), String> {
    let guard = state.store.lock().map_err(|e| e.to_string())?;
    guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.update_agent_task(&task_id, "cancelled", 0, None, None)
}

#[tauri::command]
async fn retry_agent_task(app: tauri::AppHandle, task_id: String) -> Result<String, String> {
    let (kind, payload) = {
        let state = app.state::<AppState>();
        let guard = state.store.lock().map_err(|e| e.to_string())?;
        let task = guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.get_agent_tasks(500)?.into_iter().find(|task| task.id == task_id).ok_or_else(|| "找不到任务".to_string())?;
        (task.kind, task.payload)
    };
    if kind != "embedding_rebuild" { return Err("当前只有云端 Embedding 任务支持重试".to_string()); }
    let endpoint = {
        let state = app.state::<AppState>();
        let config = state.config_store.lock().map_err(|e| e.to_string())?.get_config();
        let (enabled, endpoint) = chat::resolve_embedding_mode(&config);
        if !enabled { return Err("Embedding API 未配置".to_string()); }
        endpoint
    };
    let new_id = uuid::Uuid::new_v4().to_string();
    {
        let state = app.state::<AppState>();
        let guard = state.store.lock().map_err(|e| e.to_string())?;
        guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.create_agent_task(&new_id, "embedding_rebuild", &payload)?;
    }
    spawn_cloud_embedding_rebuild(app, new_id.clone(), endpoint);
    Ok(new_id)
}

#[tauri::command]
fn get_cloud_embedding_count(state: State<AppState>, model: Option<String>) -> Result<u64, String> {
    let guard = state.store.lock().map_err(|e| e.to_string())?;
    guard.as_ref().ok_or_else(|| "数据库未初始化".to_string())?.cloud_embedding_count(model.as_deref())
}

// ── Cloud configuration commands ──

#[tauri::command]
fn get_config(state: State<AppState>) -> Result<AppConfig, String> {
    let cs = state.config_store.lock().map_err(|e| e.to_string())?;
    Ok(cs.get_config())
}

#[tauri::command]
fn update_config(state: State<AppState>, config: AppConfig) -> Result<(), String> {
    let mut cs = state.config_store.lock().map_err(|e| e.to_string())?;
    cs.update_config(config)
}

#[tauri::command]
fn get_scan_root_path() -> Result<String, String> {
    scanner::get_scan_root().map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
fn set_scan_root(state: State<AppState>, path: Option<String>) -> Result<String, String> {
    let selected = path.map(std::path::PathBuf::from);
    let canonical = scanner::set_scan_root_override(selected)?;
    let mut config_store = state.config_store.lock().map_err(|e| e.to_string())?;
    config_store.set_scan_root(canonical.as_ref().map(|value| value.to_string_lossy().to_string()))?;
    Ok(canonical.unwrap_or(scanner::get_scan_root()?).to_string_lossy().to_string())
}

#[tauri::command]
fn set_ai_mode(state: State<AppState>, mode: String) -> Result<(), String> {
    let mut cs = state.config_store.lock().map_err(|e| e.to_string())?;
    cs.set_ai_mode(&mode)
}

#[tauri::command]
fn set_embedding_api_key(state: State<AppState>, key: String) -> Result<(), String> {
    let mut cs = state.config_store.lock().map_err(|e| e.to_string())?;
    cs.set_embedding_api_key(&key)
}

#[tauri::command]
fn set_chat_api_key(state: State<AppState>, key: String) -> Result<(), String> {
    let mut cs = state.config_store.lock().map_err(|e| e.to_string())?;
    cs.set_chat_api_key(&key)
}

#[tauri::command]
fn test_embedding_connection(state: State<AppState>) -> Result<String, String> {
    let cs = state.config_store.lock().map_err(|e| e.to_string())?;
    let config = cs.get_config();
    let ep = api_client::ApiEndpointConfig {
        base_url: config.embedding_api.base_url.clone(),
        api_key: config.embedding_api.api_key.clone(),
        model: config.embedding_api.model.clone(),
        enabled: true, // force enabled for testing
        timeout_secs: config.embedding_api.timeout_secs,
    };
    match api_client::test_embedding_connection(&ep) {
        Ok(()) => Ok("连接成功！嵌入API工作正常。".to_string()),
        Err(e) => Err(format!("连接失败: {}", e)),
    }
}

#[tauri::command]
fn test_chat_connection(state: State<AppState>) -> Result<String, String> {
    let cs = state.config_store.lock().map_err(|e| e.to_string())?;
    let config = cs.get_config();
    let ep = api_client::ApiEndpointConfig {
        base_url: config.chat_api.base_url.clone(),
        api_key: config.chat_api.api_key.clone(),
        model: config.chat_api.model.clone(),
        enabled: true,
        timeout_secs: config.chat_api.timeout_secs,
    };
    match api_client::test_chat_connection(&ep) {
        Ok(()) => Ok("连接成功！对话API工作正常。".to_string()),
        Err(e) => Err(format!("连接失败: {}", e)),
    }
}

// ── Vault Commands ──

#[tauri::command]
fn vault_configure(password: String) -> Result<(), String> {
    let vault_dir = vault::get_vault_dir()?;
    vault::configure_vault(&vault_dir, &password)
}

#[tauri::command]
fn vault_unlock(password: String, state: State<AppState>) -> Result<bool, String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;
    // Store key in memory for agent-initiated vault operations
    if let Ok(mut vk) = state.vault_key.lock() {
        *vk = Some(key);
    }
    Ok(true)
}

#[tauri::command]
fn vault_is_configured() -> Result<bool, String> {
    let vault_dir = vault::get_vault_dir()?;
    Ok(vault::is_vault_configured(&vault_dir))
}

#[tauri::command]
fn vault_add_file(file_path: String, password: String, state: State<AppState>) -> Result<vault::VaultFile, String> {
    let vault_dir = vault::get_vault_dir()?;
    // Try session key first, fall back to password
    let key = {
        let vk = state.vault_key.lock().map_err(|e| e.to_string())?;
        match *vk {
            Some(k) => k,
            None => vault::unlock_vault(&vault_dir, &password)?,
        }
    };
    let root = scanner::get_scan_root()?;
    let full_path = root.join(&file_path);
    let file_entry = vault::encrypt_file(&full_path, &vault_dir, &key)?;
    vault_update_index(&vault_dir, &key, &file_entry)?;
    Ok(file_entry)
}

#[tauri::command]
fn vault_add_external_file(path: String, password: String, state: State<AppState>) -> Result<vault::VaultFile, String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = {
        let vk = state.vault_key.lock().map_err(|e| e.to_string())?;
        match *vk {
            Some(k) => k,
            None => vault::unlock_vault(&vault_dir, &password)?,
        }
    };
    let full_path = std::path::PathBuf::from(&path);
    if !full_path.exists() {
        return Err(format!("File not found: {}", path));
    }
    let scan_root = scanner::get_scan_root()?;
    let canonical_path = std::fs::canonicalize(&full_path)
        .map_err(|e| format!("无法解析文件路径: {}", e))?;
    let canonical_root = std::fs::canonicalize(&scan_root)
        .map_err(|e| format!("无法解析扫描根路径: {}", e))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("只能添加设备内的文件".to_string());
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
    let config_store = ConfigStore::load().unwrap_or_else(|e| {
        log::error!("Failed to load config: {}, using defaults", e);
        // Create a fallback with default config
        ConfigStore::load().expect("Fatal: cannot create config store")
    });

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
            config_store: Mutex::new(config_store),
            vault_key: Mutex::new(None),
            task_queue: Mutex::new(VecDeque::new()),
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
                if let Ok(config_store) = state.config_store.lock() {
                    if let Some(root) = config_store.get_config().scan_root.as_ref() {
                        if let Err(error) = scanner::set_scan_root_override(Some(std::path::PathBuf::from(root))) {
                            log::warn!("Saved scan root is unavailable: {error}");
                        }
                    }
                }
                // API-first build: do not download or load local model weights at startup.
                // Open existing database so we don't require re-scan on restart
                if let Ok(root) = scanner::get_scan_root() {
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
                    let watcher_root = scan_root.clone();
                    if let Ok(watcher) = scanner::start_file_watcher(scan_root, move |event| {
                        watcher::sync_file_event(&app_watcher, event, watcher_root.clone());
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
            open_file_location, open_file, open_folder,
            get_files_by_category, get_files_by_tag,
            upsert_user_tag, get_user_tags, get_top_user_tags, set_file_custom_tags, get_file_custom_tags, get_files_custom_tags_batch,
            get_all_tags_with_counts, get_files_by_custom_tag, cleanup_orphan_tags,
            create_chat_session, list_chat_sessions, get_chat_messages, delete_chat_session, rename_chat_session, chat_send, stop_chat,
            execute_file_action, list_action_history, undo_action,
            get_directory, rename_file, delete_file, move_file, copy_file, import_file, create_directory,
            get_config, update_config, get_scan_root_path, set_scan_root, set_ai_mode, set_embedding_api_key, set_chat_api_key,
            test_embedding_connection, test_chat_connection, rebuild_cloud_embeddings, list_agent_tasks, cancel_agent_task, retry_agent_task, get_cloud_embedding_count,
            vault_configure, vault_unlock, vault_is_configured,
            vault_add_file, vault_add_external_file, vault_open_file, vault_delete_file,
            vault_list_files, vault_remove_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
