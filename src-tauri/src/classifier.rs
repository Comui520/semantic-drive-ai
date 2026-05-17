use crate::ai::embedding::EmbeddingEngine;
use crate::scanner::FileEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Category anchor: (name, description text for embedding)
const CATEGORY_ANCHORS: &[(&str, &str)] = &[
    ("工作文档", "工作文档 报告 合同 方案 计划书 备忘录 公文 通知 信函"),
    ("财务资料", "财务 报表 发票 账目 收支 税务 会计 审计 报销 预算"),
    ("照片", "照片 图片 风景 人物 旅游 自拍 合影 壁纸 截图 表情包"),
    ("音乐", "音乐 歌曲 专辑 歌手 歌词 音频 无损 演唱会"),
    ("视频", "视频 电影 电视剧 短片 录制 影片 剪辑 动画"),
    ("演示文稿", "演示 幻灯片 演讲 汇报 PPT 课件 答辩"),
    ("电子书", "电子书 阅读 小说 书籍 文献 笔记 教程 百科"),
    ("代码文件", "代码 程序 编程 脚本 源码 开发 算法 数据库 配置"),
    ("压缩包", "压缩包 备份 打包 归档"),
    ("设计文件", "设计 图层 矢量 模型 UI UX 平面 3D"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub name: String,
    pub label: String,
    pub count: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    pub categories: Vec<CategoryInfo>,
    pub tags: Vec<TagInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub count: u64,
}

/// Extract the year (first 4 chars) from a modified timestamp string, safely.
fn extract_year(modified: &str) -> Option<u32> {
    modified.get(..4).and_then(|s| s.parse::<u32>().ok())
}

/// Generate per-file tags from filename and metadata (comma-separated).
pub fn generate_tags_for_file(file: &FileEntry) -> String {
    let mut tags: Vec<String> = Vec::new();
    if let Some(year) = extract_year(&file.modified) {
        tags.push(format!("{}年", year));
    }
    if !file.extension.is_empty() {
        tags.push(file.extension.to_uppercase());
    }
    for part in file.name.split(['_', '-', ' ', '.', '（', '）', '(', ')']) {
        let part = part.trim();
        if part.len() >= 2 && !part.chars().all(|c| c.is_ascii_digit()) {
            let s = part.to_string();
            if !tags.contains(&s) {
                tags.push(s);
            }
        }
    }
    tags.join(",")
}

/// Batch-classify all files, returning aggregate result + per-file categories and tags.
///
/// Embeds anchors once, then embeds file signatures in chunks of 128 to avoid OOM.
/// If the embedding engine returns None (model not loaded), falls back to extension-only.
pub fn classify_files_batch(
    files: &[FileEntry],
    engine: &EmbeddingEngine,
) -> (ClassificationResult, Vec<(String, String)>, Vec<(String, String)>) {
    // 1. Batch-embed all 10 anchor descriptions (1 forward pass)
    let anchor_texts: Vec<String> = CATEGORY_ANCHORS.iter().map(|(_, d)| d.to_string()).collect();
    let anchor_embs = engine.embed_batch(&anchor_texts);

    // 2. Build text signatures for all files
    let signatures: Vec<String> = files
        .iter()
        .map(|f| format!("{} {}", f.name, f.path.replace('/', " ")))
        .collect();

    // 3. Batch-embed signatures in chunks of 128 to avoid OOM
    let mut file_embs: Vec<Vec<f32>> = Vec::with_capacity(signatures.len());
    for chunk in signatures.chunks(32) {
        file_embs.extend(engine.embed_batch(chunk));
    }

    // 4. Compare each file embedding against anchors (pure math)
    let mut categories: HashMap<String, (u64, u64)> = HashMap::new();
    let mut tag_counts: HashMap<String, u64> = HashMap::new();
    let mut per_file_cats: Vec<(String, String)> = Vec::with_capacity(files.len());
    let mut per_file_tags: Vec<(String, String)> = Vec::with_capacity(files.len());

    for (i, file) in files.iter().enumerate() {
        let file_emb = &file_embs[i];

        let mut best_score = 0.25f32;
        let mut best_cat = extension_fallback(&file.extension);

        if !file_emb.is_empty() {
            for (j, (cat_name, _)) in CATEGORY_ANCHORS.iter().enumerate() {
                let score = EmbeddingEngine::cosine_similarity(file_emb, &anchor_embs[j]);
                if score > best_score {
                    best_score = score;
                    best_cat = cat_name;
                }
            }
        }

        let (count, size) = categories.entry(best_cat.to_string()).or_insert((0, 0));
        *count += 1;
        *size += file.size;

        per_file_cats.push((file.id.clone(), best_cat.to_string()));

        // Generate tags from filename and path
        let mut file_tags: Vec<String> = Vec::new();
        for part in file.name.split(['_', '-', ' ', '.', '（', '）', '(', ')']) {
            let part = part.trim();
            if part.len() >= 2 && !part.chars().all(|c| c.is_ascii_digit()) {
                *tag_counts.entry(part.to_string()).or_insert(0) += 1;
                if !file_tags.contains(&part.to_string()) {
                    file_tags.push(part.to_string());
                }
            }
        }

        // Year-based tags (safe slicing)
        if let Some(year) = extract_year(&file.modified) {
            let yt = format!("{}年", year);
            *tag_counts.entry(yt.clone()).or_insert(0) += 1;
            file_tags.push(yt);
        }

        // Extension tag
        if !file.extension.is_empty() {
            let et = file.extension.to_uppercase();
            *tag_counts.entry(et.clone()).or_insert(0) += 1;
            file_tags.push(et);
        }

        per_file_tags.push((file.id.clone(), file_tags.join(",")));
    }

    let mut category_list: Vec<CategoryInfo> = categories
        .into_iter()
        .map(|(name, (count, total_size))| CategoryInfo {
            name: name.clone(),
            label: name,
            count,
            total_size,
        })
        .collect();
    category_list.sort_by(|a, b| b.count.cmp(&a.count));

    let mut tag_list: Vec<(String, u64)> = tag_counts.into_iter().collect();
    tag_list.sort_by(|a, b| b.1.cmp(&a.1));
    let tags: Vec<TagInfo> = tag_list
        .into_iter()
        .take(50)
        .map(|(name, count)| TagInfo { name, count })
        .collect();

    (ClassificationResult { categories: category_list, tags }, per_file_cats, per_file_tags)
}

/// Extension-based fallback mapping.
fn extension_fallback(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" => "工作文档",
        "xls" | "xlsx" | "csv" | "tsv" => "财务资料",
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" | "raw" | "heic" => "照片",
        "mp3" | "wav" | "flac" | "aac" | "ogg" => "音乐",
        "mp4" | "avi" | "mkv" | "mov" | "webm" | "flv" => "视频",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" => "压缩包",
        "ppt" | "pptx" | "key" => "演示文稿",
        "epub" | "mobi" | "azw3" => "电子书",
        "psd" | "ai" | "svg" | "fig" | "sketch" => "设计文件",
        "c" | "cpp" | "h" | "rs" | "py" | "js" | "ts" | "java" | "go" | "rb" | "php" => "代码文件",
        _ => "其他",
    }
}
