mod ai;
mod classifier;
mod dedup;
mod scanner;
mod store;
mod vault;

use ai::llm::LlmEngine;
use ai::model_manager::ModelInfo;
use ai::search::{SearchEngine, SearchResult};
use classifier::ClassificationResult;
use dedup::DuplicateGroup;
use scanner::FileEntry;
use store::MetadataStore;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

struct AppState {
    store: Mutex<Option<MetadataStore>>,
    device_root: Mutex<Option<String>>,
    search_engine: Mutex<SearchEngine>,
    llm_engine: Mutex<LlmEngine>,
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
    let root = scanner::get_device_root().map_err(|e| format!("Cannot get device root: {}", e))?;

    {
        let state = app.state::<AppState>();
        let mut root_lock = state.device_root.lock().map_err(|e| e.to_string())?;
        *root_lock = Some(root.to_string_lossy().to_string());
    }

    log::info!("Scanning: {:?}", root);
    let app_cb = app.clone();
    let root_cb = root.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let mut entries = scanner::scan_directory(&root_cb)?;
        let state = app_cb.state::<AppState>();
        if let Ok(mut engine) = state.search_engine.lock() {
            for entry in &mut entries {
                let full_path = root_cb.join(&entry.path);
                if let Ok(text) = ai::extract_text(&full_path, &entry.extension) {
                    if text.len() > 100 {
                        engine.index_file(&entry.id, &text);
                        log::debug!("Indexed {} chars from {}", text.len(), entry.name);
                    }
                }
            }
        }
        Ok::<Vec<FileEntry>, String>(entries)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    let data_dir = root.join(".semanticdrive");
    let app_db = app.clone();
    let entries_db = entries.clone();
    let count = tokio::task::spawn_blocking(move || {
        let db = MetadataStore::open(&data_dir)?;
        let count = db.replace_all(&entries_db)?;
        log::info!("Indexed {} files", count);
        let state = app_db.state::<AppState>();
        if let Ok(mut store_lock) = state.store.lock() {
            *store_lock = Some(db);
        }
        Ok::<usize, String>(count)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    log::info!("Indexed {} files", count);
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
fn get_file_count(state: State<AppState>) -> Result<u64, String> {
    let store_lock = state.store.lock().map_err(|e| e.to_string())?;
    match store_lock.as_ref() {
        Some(db) => db.file_count(),
        None => Ok(0),
    }
}

#[tauri::command]
fn compute_file_hash(file_path: String) -> Result<String, String> {
    let root = scanner::get_device_root()?;
    let full_path = root.join(&file_path);
    scanner::compute_hash(&full_path)
        .ok_or_else(|| format!("Cannot hash file: {}", file_path))
}

// ── Search Commands ──

#[tauri::command]
async fn search_files(
    app: tauri::AppHandle,
    query: String,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    // Step 1: Parse natural language query with LLM (or fallback)
    let app_clone = app.clone();
    let query_clone = query.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        let state = app_clone.state::<AppState>();
        let mut llm = state.llm_engine.lock().map_err(|e| e.to_string())?;
        Ok::<_, String>(llm.parse_query(&query_clone))
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    log::info!(
        "Search: keywords={:?}, time_range={:?}, types={:?}, entities={:?}",
        parsed.keywords, parsed.time_range, parsed.file_types, parsed.entities
    );

    // Step 2: Use keywords (or original query) for semantic search
    let search_query = if parsed.keywords.is_empty() {
        query.clone()
    } else {
        parsed.keywords.join(" ")
    };

    // Step 3-5: Run semantic search, apply filters, sort (all in blocking thread)
    let app_clone = app.clone();
    let search_query_clone = search_query.clone();
    let time_range = parsed.time_range.clone();
    let file_types = parsed.file_types.clone();
    let entities = parsed.entities.clone();
    let max_results_clamped = max_results.max(20).min(200);

    let results = tokio::task::spawn_blocking(move || {
        let state = app_clone.state::<AppState>();
        let engine = state.search_engine.lock().map_err(|e| e.to_string())?;
        let files = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
            Some(db) => db.get_all_files().unwrap_or_default(),
            None => Vec::new(),
        };

        let mut results = engine.search(&search_query_clone, &files, max_results_clamped * 2);

        // Time range filter
        if let Some((ref start, ref end)) = time_range {
            results.retain(|r| r.modified >= *start && r.modified <= *end);
        }
        // File type filter
        if !file_types.is_empty() {
            results.retain(|r| {
                file_types
                    .iter()
                    .any(|t| r.file_path.ends_with(t) || r.file_name.to_lowercase().ends_with(t))
            });
        }
        // Entity match boosting
        for r in &mut results {
            for entity in &entities {
                if r.file_name.contains(entity) || r.snippet.contains(entity) {
                    r.score = (r.score + 0.2).min(1.0);
                    r.match_type = "语义+实体匹配".to_string();
                }
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(max_results_clamped);
        Ok::<Vec<SearchResult>, String>(results)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))??;

    Ok(results)
}

#[tauri::command]
fn index_file_content(
    file_id: String,
    content: String,
    state: State<AppState>,
) -> Result<(), String> {
    let mut engine = state.search_engine.lock().map_err(|e| e.to_string())?;
    engine.index_file(&file_id, &content);
    Ok(())
}

#[tauri::command]
fn get_indexed_count(state: State<AppState>) -> Result<usize, String> {
    let engine = state.search_engine.lock().map_err(|e| e.to_string())?;
    Ok(engine.indexed_count())
}

// ── Classification Commands ──

#[tauri::command]
async fn classify_files(app: tauri::AppHandle) -> Result<ClassificationResult, String> {
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let state = app_clone.state::<AppState>();
        let files = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
            Some(db) => db.get_all_files().unwrap_or_default(),
            None => return Err("No files indexed. Run scan first.".to_string()),
        };
        let engine = state.search_engine.lock().map_err(|e| e.to_string())?;
        Ok::<ClassificationResult, String>(classifier::classify_files(&files, engine.embedding_engine()))
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

// ── Dedup Commands ──

#[tauri::command]
async fn find_duplicates(app: tauri::AppHandle) -> Result<Vec<DuplicateGroup>, String> {
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let state = app_clone.state::<AppState>();
        let files = match state.store.lock().map_err(|e| e.to_string())?.as_ref() {
            Some(db) => db.get_all_files().unwrap_or_default(),
            None => return Err("No files indexed. Run scan first.".to_string()),
        };
        let root = scanner::get_device_root()?;
        Ok::<Vec<DuplicateGroup>, String>(dedup::find_duplicates(&files, &root))
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

// ── Model Management Commands ──

#[tauri::command]
fn get_models_status() -> Result<Vec<ModelInfo>, String> {
    ai::model_manager::get_models_status()
}

#[tauri::command]
async fn download_model_file(app: tauri::AppHandle, model_id: String) -> Result<String, String> {
    let app_for_cb = app.clone();
    let app_for_post = app.clone();
    let model_id_cb = model_id.clone();

    // Download in blocking thread so UI stays responsive
    let path = tokio::task::spawn_blocking(move || {
        let cb = move |progress: ai::model_manager::DownloadProgress| {
            log::info!(
                "Download {}: {:.1}%",
                progress.model_id,
                progress.progress_pct
            );
            let _ = app_for_cb.emit("download-progress", &progress);
        };
        ai::model_manager::download_model(&model_id_cb, cb)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))??;

    // After successful download, load model into engine
    let path_clone = path.clone();
    tokio::task::spawn_blocking(move || {
        #[cfg(not(mobile))]
        {
            let state = app_for_post.state::<AppState>();
            match model_id.as_str() {
                "bge-small-zh" => {
                    if let Ok(mut engine) = state.search_engine.lock() {
                        if let Err(e) = engine.load_embedding_model(&path_clone) {
                            log::error!("Failed to load embedding model after download: {}", e);
                        } else {
                            log::info!("Embedding model loaded after download");
                        }
                    }
                }
                "qwen2.5-0.5b" => {
                    if let Ok(mut engine) = state.llm_engine.lock() {
                        if let Err(e) = engine.load(&path_clone) {
                            log::error!("Failed to load LLM after download: {}", e);
                        } else {
                            log::info!("LLM loaded after download");
                        }
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?;

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
    match vault::unlock_vault(&vault_dir, &password) {
        Ok(_) => Ok(true),
        Err(e) => Err(e),
    }
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
    let root = scanner::get_device_root()?;
    let full_path = root.join(&file_path);
    vault::encrypt_file(&full_path, &vault_dir, &key)
}

#[tauri::command]
fn vault_list_files(password: String) -> Result<Vec<vault::VaultFile>, String> {
    let vault_dir = vault::get_vault_dir()?;
    let key = vault::unlock_vault(&vault_dir, &password)?;

    // Read index file
    let index_path = vault_dir.join("files").join("index.json");
    if !index_path.exists() {
        return Ok(Vec::new());
    }

    let encrypted = std::fs::read(&index_path)
        .map_err(|e| format!("Cannot read vault index: {}", e))?;
    let decrypted = vault::decrypt_data(&encrypted, &key)?;
    let files: Vec<vault::VaultFile> =
        serde_json::from_slice(&decrypted).map_err(|e| format!("Cannot parse vault index: {}", e))?;
    Ok(files)
}

#[tauri::command]
fn vault_remove_file(encrypted_name: String) -> Result<(), String> {
    let vault_dir = vault::get_vault_dir()?;
    vault::remove_vault_file(&encrypted_name, &vault_dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            store: Mutex::new(None),
            device_root: Mutex::new(None),
            search_engine: Mutex::new(SearchEngine::new()),
            llm_engine: Mutex::new(LlmEngine::new()),
        })
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Auto-load AI models if already downloaded
            #[cfg(not(mobile))]
            {
                let state = app.state::<AppState>();
                // Load embedding model (BGE)
                if let Ok(Some(path)) = ai::model_manager::get_model_path("bge-small-zh") {
                    log::info!("Auto-loading embedding model from: {:?}", path);
                    if let Ok(mut engine) = state.search_engine.lock() {
                        if let Err(e) = engine.load_embedding_model(&path) {
                            log::warn!("Failed to load embedding model: {}", e);
                        } else {
                            log::info!("Embedding model loaded successfully");
                        }
                    }
                }
                // Load LLM (Qwen2.5)
                if let Ok(Some(path)) = ai::model_manager::get_model_path("qwen2.5-0.5b") {
                    log::info!("Auto-loading LLM from: {:?}", path);
                    if let Ok(mut engine) = state.llm_engine.lock() {
                        if let Err(e) = engine.load(&path) {
                            log::warn!("Failed to load LLM: {}", e);
                        } else {
                            log::info!("LLM loaded successfully");
                        }
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_device_root,
            scan_files,
            get_files,
            get_file_count,
            compute_file_hash,
            search_files,
            index_file_content,
            get_indexed_count,
            classify_files,
            find_duplicates,
            get_models_status,
            download_model_file,
            vault_configure,
            vault_unlock,
            vault_is_configured,
            vault_add_file,
            vault_list_files,
            vault_remove_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
