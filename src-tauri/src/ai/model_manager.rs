use serde::{Deserialize, Serialize};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// Model definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub url: String,
    pub target_dir: String,
    pub target_file: String,
    pub expected_size_mb: u64,
    pub is_downloaded: bool,
}

/// Download progress event payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub progress_pct: f32,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelFile {
    filename: String,
    /// Remote filename on HuggingFace (defaults to same as local if None)
    remote_name: Option<String>,
    /// HuggingFace repo in "org/name" format
    repo: &'static str,
    /// Relative size ranking (for progress bar weighting)
    size_rank: u32,
}

/// Primary HuggingFace URL (may be slow or blocked in China)
const HF_PRIMARY: &str = "https://huggingface.co";
/// Mirror for China users (hf-mirror is the official HF mirror)
const HF_MIRROR: &str = "https://hf-mirror.com";

const MODELS_DIR: &str = ".semanticdrive/models";

fn models_dir() -> Result<PathBuf, String> {
    let root = crate::scanner::get_device_root()?;
    Ok(root.join(MODELS_DIR))
}

/// Define the files needed for each logical model.
/// Returns (model_id, required_files, optional_files).
fn model_file_sets() -> Vec<(&'static str, Vec<ModelFile>, Vec<ModelFile>)> {
    vec![
        (
            "bge-small-zh",
            vec![
                ModelFile {
                    filename: "config.json".to_string(),
                    remote_name: None,
                    repo: "BAAI/bge-small-zh-v1.5",
                    size_rank: 1,
                },
                ModelFile {
                    filename: "tokenizer.json".to_string(),
                    remote_name: None,
                    repo: "BAAI/bge-small-zh-v1.5",
                    size_rank: 1,
                },
                ModelFile {
                    filename: "model.safetensors".to_string(),
                    remote_name: None,
                    repo: "BAAI/bge-small-zh-v1.5",
                    size_rank: 98,
                },
            ],
            vec![],
        ),
        (
            "qwen2.5-0.5b",
            vec![
                ModelFile {
                    filename: "model.gguf".to_string(),
                    remote_name: Some("Qwen2.5-0.5B.Q4_K_M.gguf".to_string()),
                    repo: "mradermacher/Qwen2.5-0.5B-GGUF",
                    size_rank: 99,
                },
                ModelFile {
                    filename: "tokenizer.json".to_string(),
                    remote_name: None,
                    repo: "Qwen/Qwen2.5-0.5B",
                    size_rank: 1,
                },
            ],
            vec![],
        ),
    ]
}

/// Return the list of all required models with download status.
pub fn get_models_status() -> Result<Vec<ModelInfo>, String> {
    let base = models_dir()?;
    let mut results = Vec::new();

    for (model_id, required, _optional) in model_file_sets() {
        let model_dir = base.join(model_id);
        let all_exist: bool = required.iter().all(|f| {
            let path = model_dir.join(&f.filename);
            path.exists() && path.metadata().map(|md| md.len()).unwrap_or(0) > 0
        });

        let (name, desc, size_mb) = match model_id {
            "bge-small-zh" => (
                "BGE-small-zh 嵌入模型",
                "用于文本向量化，实现语义搜索和智能分类",
                96,
            ),
            "qwen2.5-0.5b" => (
                "Qwen2.5 语言模型",
                "用于自然语言查询理解和意图解析",
                398,
            ),
            _ => continue,
        };

        results.push(ModelInfo {
            id: model_id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            url: format!("{}/{}/tree/main", HF_PRIMARY, required[0].repo),
            target_dir: model_id.to_string(),
            target_file: required.last().map(|f| f.filename.clone()).unwrap_or_default(),
            expected_size_mb: size_mb,
            is_downloaded: all_exist,
        });
    }

    Ok(results)
}

pub fn all_models_ready() -> Result<bool, String> {
    let models = get_models_status()?;
    Ok(models.iter().all(|m| m.is_downloaded))
}

pub fn is_model_ready(model_id: &str) -> Result<bool, String> {
    let base = models_dir()?;
    let sets = model_file_sets();
    let (_id, required, _optional) = match sets.iter().find(|(id, _, _)| *id == model_id) {
        Some(f) => f,
        None => return Ok(false),
    };
    let model_dir = base.join(model_id);
    Ok(required.iter().all(|f| {
        let path = model_dir.join(&f.filename);
        path.exists() && path.metadata().map(|md| md.len()).unwrap_or(0) > 0
    }))
}

pub fn get_model_path(model_id: &str) -> Result<Option<PathBuf>, String> {
    let base = models_dir()?;
    let model_dir = base.join(model_id);
    let sets = model_file_sets();
    let (_id, required, _optional) = match sets.iter().find(|(id, _, _)| *id == model_id) {
        Some(f) => f,
        None => return Ok(None),
    };
    let all_exist = required.iter().all(|f| {
        let path = model_dir.join(&f.filename);
        path.exists() && path.metadata().map(|md| md.len()).unwrap_or(0) > 0
    });
    if all_exist { Ok(Some(model_dir)) } else { Ok(None) }
}

/// Download a specific model (all its files) with progress callbacks.
/// Tries primary URL first, falls back to hf-mirror on connection failure.
pub fn download_model(
    model_id: &str,
    progress_callback: impl Fn(DownloadProgress),
) -> Result<PathBuf, String> {
    match model_id {
        "bge-small-zh" | "qwen2.5-0.5b" => download_model_files(model_id, progress_callback),
        _ => Err(format!("未知模型: {}", model_id)),
    }
}

fn download_model_files(
    model_id: &str,
    progress_callback: impl Fn(DownloadProgress),
) -> Result<PathBuf, String> {
    let base = models_dir()?;
    let target_dir = base.join(model_id);
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("无法创建模型目录: {}", e))?;

    let sets = model_file_sets();
    let (_id, required, optional) = sets
        .iter()
        .find(|(id, _, _)| *id == model_id)
        .ok_or_else(|| format!("未知模型: {}", model_id))?;

    // Build the full download list: required + optional (best-effort)
    let all_files: Vec<&ModelFile> = required.iter().chain(optional.iter()).collect();
    let optional_set: std::collections::HashSet<&str> =
        optional.iter().map(|f| f.filename.as_str()).collect();

    let total_weight: u32 = all_files.iter().map(|f| f.size_rank).sum();
    let mut accumulated_weight: u32 = 0;

    for file_entry in all_files {
        let is_optional = optional_set.contains(file_entry.filename.as_str());
        let file_path = target_dir.join(&file_entry.filename);
        let part_path = target_dir.join(format!("{}.part", &file_entry.filename));

        // Skip already-downloaded files
        if file_path.exists() && file_path.metadata().map(|md| md.len()).unwrap_or(0) > 0 {
            log::info!("跳过已存在的文件: {}", file_entry.filename);
            accumulated_weight += file_entry.size_rank;
            continue;
        }

        // Clean up any previous partial download
        let _ = std::fs::remove_file(&part_path);

        let remote_name = file_entry.remote_name.as_deref().unwrap_or(&file_entry.filename);
        let primary_url = format!(
            "{}/{}/resolve/main/{}?download=true",
            HF_PRIMARY, file_entry.repo, remote_name
        );
        let mirror_url = format!(
            "{}/{}/resolve/main/{}?download=true",
            HF_MIRROR, file_entry.repo, remote_name
        );

        let before_pct = (accumulated_weight as f32 / total_weight as f32) * 100.0;
        let file_weight_pct = (file_entry.size_rank as f32 / total_weight as f32) * 100.0;

        log::info!("下载 {}：{}", model_id, file_entry.filename);

        // Report progress at start of this file
        progress_callback(DownloadProgress {
            model_id: model_id.to_string(),
            bytes_downloaded: 0,
            total_bytes: 100,
            progress_pct: before_pct,
            status: "downloading".to_string(),
            error: None,
        });

        // Try primary, then mirror (download to .part file)
        let result = try_download_with_progress(
            &primary_url,
            &part_path,
            &file_entry.filename,
            file_weight_pct,
            before_pct,
            &progress_callback,
        );

        let result = match result {
            Ok(bytes) => {
                log::info!("{} 下载完成: {} bytes", file_entry.filename, bytes);
                // Rename .part to final filename
                std::fs::rename(&part_path, &file_path)
                    .map_err(|e| format!("无法重命名文件: {}", e))?;
                accumulated_weight += file_entry.size_rank;
                Ok(bytes)
            }
            Err(e) => {
                log::warn!("主站下载失败，尝试镜像: {}", e);
                let _ = std::fs::remove_file(&part_path);
                try_download_with_progress(
                    &mirror_url,
                    &part_path,
                    &file_entry.filename,
                    file_weight_pct,
                    before_pct,
                    &progress_callback,
                )
                .map_err(|mirror_e| {
                    format!(
                        "文件 '{}' 下载失败。\n主站错误: {}\n镜像错误: {}",
                        file_entry.filename, e, mirror_e
                    )
                })
            }
        };

        if let Err(err) = result {
            let _ = std::fs::remove_file(&part_path);
            if is_optional {
                // Optional file failure is not fatal — skip it
                log::warn!("可选文件 '{}' 下载失败，已跳过: {}", file_entry.filename, err);
                progress_callback(DownloadProgress {
                    model_id: model_id.to_string(),
                    bytes_downloaded: 0,
                    total_bytes: 0,
                    progress_pct: before_pct + file_weight_pct, // advance progress anyway
                    status: "downloading".to_string(),
                    error: None,
                });
                accumulated_weight += file_entry.size_rank;
            } else {
                progress_callback(DownloadProgress {
                    model_id: model_id.to_string(),
                    bytes_downloaded: 0,
                    total_bytes: 0,
                    progress_pct: before_pct,
                    status: "error".to_string(),
                    error: Some(err.clone()),
                });
                return Err(err);
            }
        }

    }

    progress_callback(DownloadProgress {
        model_id: model_id.to_string(),
        bytes_downloaded: 100,
        total_bytes: 100,
        progress_pct: 100.0,
        status: "completed".to_string(),
        error: None,
    });

    Ok(target_dir)
}

/// Download a single file, calling progress_fn during download.
fn try_download_with_progress(
    url: &str,
    dest: &Path,
    file_label: &str,
    file_weight_pct: f32,
    base_pct: f32,
    progress_cb: &impl Fn(DownloadProgress),
) -> Result<u64, String> {
    let model_id = dest
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|p| p.to_str())
        .unwrap_or("unknown")
        .to_string();

    let response = ureq::get(url)
        .header("User-Agent", "SemanticDriveAI/1.0")
        .call()
        .map_err(|e| format_error(&e))?;

    if response.status().as_u16() != 200 {
        return Err(format!("HTTP {}", response.status()));
    }

    let total_size: u64 = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("无法创建文件 {}: {}", file_label, e))?;

    let mut buffered = BufReader::with_capacity(64 * 1024, reader);
    let mut buf = vec![0u8; 64 * 1024];
    let mut total_downloaded: u64 = 0;

    // Report progress every ~2% or 256KB
    let report_interval = if total_size > 0 {
        (total_size / 50).max(256 * 1024)
    } else {
        512 * 1024
    };
    let mut next_report = report_interval;

    loop {
        let n = buffered
            .read(&mut buf)
            .map_err(|e| format!("读取下载流失败: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入文件失败: {}", e))?;
        total_downloaded += n as u64;

        if total_downloaded >= next_report {
            let file_progress = if total_size > 0 {
                (total_downloaded as f32 / total_size as f32) * file_weight_pct
            } else {
                0.0
            };
            progress_cb(DownloadProgress {
                model_id: model_id.clone(),
                bytes_downloaded: total_downloaded,
                total_bytes: total_size,
                progress_pct: base_pct + file_progress,
                status: "downloading".to_string(),
                error: None,
            });
            next_report = total_downloaded + report_interval;
        }
    }

    // Final 100% for this file
    progress_cb(DownloadProgress {
        model_id,
        bytes_downloaded: total_downloaded,
        total_bytes: total_size,
        progress_pct: base_pct + file_weight_pct,
        status: "downloading".to_string(),
        error: None,
    });

    Ok(total_downloaded)
}

/// Format ureq errors into readable messages.
fn format_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(s) => {
            format!("服务器返回 HTTP {} (文件可能不存在或链接已失效)", s)
        }
        ureq::Error::HostNotFound => "DNS解析失败，huggingface.co 可能被屏蔽".to_string(),
        ureq::Error::ConnectionFailed => "无法连接到服务器，网络不通或被屏蔽".to_string(),
        ureq::Error::Timeout(t) => format!("连接超时: {}", t),
        ureq::Error::Tls(msg) => format!("TLS/SSL错误: {}", msg),
        ureq::Error::Io(e) => format!("网络IO错误: {}", e),
        _ => format!("下载错误: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_file_sets_contains_bge() {
        let sets = model_file_sets();
        let bge = sets.iter().find(|(id, _, _)| *id == "bge-small-zh");
        assert!(bge.is_some(), "bge-small-zh should be in model sets");
    }

    #[test]
    fn test_model_file_sets_contains_qwen() {
        let sets = model_file_sets();
        let qwen = sets.iter().find(|(id, _, _)| *id == "qwen2.5-0.5b");
        assert!(qwen.is_some(), "qwen2.5-0.5b should be in model sets");
    }

    #[test]
    fn test_bge_has_required_files() {
        let sets = model_file_sets();
        let (_id, required, _optional) = sets.iter()
            .find(|(id, _, _)| *id == "bge-small-zh")
            .expect("bge model should exist");
        assert_eq!(required.len(), 3, "bge should have 3 required files");
        assert!(required.iter().any(|f| f.filename == "config.json"));
        assert!(required.iter().any(|f| f.filename == "tokenizer.json"));
        assert!(required.iter().any(|f| f.filename == "model.safetensors"));
    }

    #[test]
    fn test_qwen_has_required_files() {
        let sets = model_file_sets();
        let (_id, required, _optional) = sets.iter()
            .find(|(id, _, _)| *id == "qwen2.5-0.5b")
            .expect("qwen model should exist");
        assert_eq!(required.len(), 2, "qwen should have 2 required files");
        assert!(required.iter().any(|f| f.filename == "model.gguf"));
        assert!(required.iter().any(|f| f.filename == "tokenizer.json"));
    }

    #[test]
    fn test_bge_files_have_repos() {
        let sets = model_file_sets();
        let (_id, required, optional) = sets.iter()
            .find(|(id, _, _)| *id == "bge-small-zh")
            .expect("bge model should exist");
        for f in required.iter().chain(optional.iter()) {
            assert!(!f.repo.is_empty(), "repo should not be empty for {}", f.filename);
        }
    }

    #[test]
    fn test_qwen_files_have_repos() {
        let sets = model_file_sets();
        let (_id, required, optional) = sets.iter()
            .find(|(id, _, _)| *id == "qwen2.5-0.5b")
            .expect("qwen model should exist");
        for f in required.iter().chain(optional.iter()) {
            assert!(!f.repo.is_empty(), "repo should not be empty for {}", f.filename);
        }
    }

    #[test]
    fn test_total_weight_positive() {
        let sets = model_file_sets();
        for (model_id, required, optional) in &sets {
            let total: u32 = required.iter().chain(optional.iter()).map(|f| f.size_rank).sum();
            assert!(total > 0, "total weight should be positive for {}", model_id);
        }
    }

    #[test]
    fn test_get_models_status_returns_all() {
        let result = get_models_status().unwrap();
        assert_eq!(result.len(), 2, "should return 2 models");
        let bge = result.iter().find(|m| m.id == "bge-small-zh");
        let qwen = result.iter().find(|m| m.id == "qwen2.5-0.5b");
        assert!(bge.is_some(), "bge should be in status");
        assert!(qwen.is_some(), "qwen should be in status");
    }

    #[test]
    fn test_model_info_contains_url() {
        let result = get_models_status().unwrap();
        for model in &result {
            assert!(!model.url.is_empty(), "url should not be empty for {}", model.id);
            assert!(model.url.starts_with("http"), "url should start with http for {}", model.id);
        }
    }

    #[test]
    fn test_models_not_downloaded_by_default() {
        // Since there's no actual model directory in test environment
        let result = get_models_status().unwrap();
        // Both should NOT be downloaded (no models in temp test env)
        // Note: this might fail if tests run from a directory that has models
        for model in &result {
            assert!(!model.is_downloaded, "{} should not be downloaded in test env", model.id);
        }
    }

    #[test]
    fn test_known_model_sizes_positive() {
        let result = get_models_status().unwrap();
        for model in &result {
            assert!(model.expected_size_mb > 0, "size should be positive for {}", model.id);
        }
    }
}
