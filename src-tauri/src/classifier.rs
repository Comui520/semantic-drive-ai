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

/// Classify files using embedding-based similarity and extension fallback.
pub fn classify_files(
    files: &[FileEntry],
    embedding_engine: &EmbeddingEngine,
) -> ClassificationResult {
    // Pre-compute category anchor embeddings
    let anchor_embeddings: Vec<(&str, Vec<f32>)> = CATEGORY_ANCHORS
        .iter()
        .map(|(name, desc)| (*name, embedding_engine.embed(desc)))
        .collect();

    let mut categories: HashMap<String, (u64, u64)> = HashMap::new();
    let mut tag_counts: HashMap<String, u64> = HashMap::new();

    for file in files {
        // Determine category: embedding-based with extension fallback
        let cat = classify_file(file, &anchor_embeddings, embedding_engine);

        let (count, size) = categories.entry(cat.to_string()).or_insert((0, 0));
        *count += 1;
        *size += file.size;

        // Generate tags from filename and path
        for part in file.name.split(['_', '-', ' ', '.', '（', '）', '(', ')']) {
            let part = part.trim();
            if part.len() >= 2 && !part.chars().all(|c| c.is_ascii_digit()) {
                *tag_counts.entry(part.to_string()).or_insert(0) += 1;
            }
        }

        // Year-based tags
        if let Some(year) = &file.modified[..4].parse::<u32>().ok() {
            *tag_counts.entry(format!("{}年", year)).or_insert(0) += 1;
        }

        // Extension tag
        if !file.extension.is_empty() {
            *tag_counts
                .entry(file.extension.to_uppercase())
                .or_insert(0) += 1;
        }
    }

    // Build categories sorted by count
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

    // Build tags sorted by count (top 50)
    let mut tag_list: Vec<(String, u64)> = tag_counts.into_iter().collect();
    tag_list.sort_by(|a, b| b.1.cmp(&a.1));
    let tags: Vec<TagInfo> = tag_list
        .into_iter()
        .take(50)
        .map(|(name, count)| TagInfo { name, count })
        .collect();

    ClassificationResult {
        categories: category_list,
        tags,
    }
}

/// Classify a single file by comparing its name+path embedding to category anchors.
fn classify_file<'a>(
    file: &FileEntry,
    anchors: &'a [(&'a str, Vec<f32>)],
    engine: &EmbeddingEngine,
) -> &'a str {
    // Build a text signature from filename and path
    let signature = format!("{} {}", file.name, file.path.replace('/', " ").replace('\\', " "));
    let file_emb = engine.embed(&signature);

    // Find best anchor match
    let mut best_score = 0.0f32;
    let mut best_cat = extension_fallback(&file.extension);

    for (cat_name, anchor_emb) in anchors {
        let score = EmbeddingEngine::cosine_similarity(&file_emb, anchor_emb);
        if score > best_score {
            best_score = score;
            best_cat = cat_name;
        }
    }

    // Only use embedding-based classification if confidence is reasonable
    // Otherwise fall back to extension-based (more predictable)
    if best_score > 0.25 {
        best_cat
    } else {
        extension_fallback(&file.extension)
    }
}

/// Extension-based fallback mapping (same as before but internal).
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
