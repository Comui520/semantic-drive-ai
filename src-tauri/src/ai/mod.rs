pub mod embedding;
pub mod extractor;
pub mod llm;
pub mod model_manager;
pub mod search;

use std::path::Path;

/// Supported file types for content extraction
#[derive(Debug, Clone, PartialEq)]
pub enum FileCategory {
    Document, // pdf, docx, txt
    Spreadsheet, // xlsx, csv
    Image,    // jpg, png (needs OCR)
    Media,    // mp3, mp4
    Archive,  // zip, rar
    Other,
}

impl FileCategory {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" => FileCategory::Document,
            "xls" | "xlsx" | "csv" | "tsv" => FileCategory::Spreadsheet,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" => FileCategory::Image,
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "mp4" | "avi" | "mkv" | "mov" | "webm" => {
                FileCategory::Media
            }
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" => FileCategory::Archive,
            _ => FileCategory::Other,
        }
    }

    pub fn category_label(&self) -> &str {
        match self {
            FileCategory::Document => "文档",
            FileCategory::Spreadsheet => "表格",
            FileCategory::Image => "图片",
            FileCategory::Media => "媒体",
            FileCategory::Archive => "压缩包",
            FileCategory::Other => "其他",
        }
    }
}

/// Extract text content from a file based on its extension
pub fn extract_text(file_path: &Path, extension: &str) -> Result<String, String> {
    match extension.to_lowercase().as_str() {
        "txt" | "md" | "csv" | "tsv" | "json" | "xml" | "html" | "htm" | "log"
        | "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "go" | "java" | "kt" | "dart"
        | "cpp" | "c" | "h" | "hpp" | "cs" | "rb" | "php" | "swift" | "scala" | "r"
        | "sh" | "bat" | "ps1" | "pl" | "lua" | "sql" | "vue" | "svelte" | "astro"
        | "css" | "scss" | "less" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf"
        | "gradle" | "makefile" | "dockerfile" | "cmake" | "m" | "mm" => {
            extractor::extract_text_plain(file_path)
        }
        "pdf" => extractor::extract_text_pdf(file_path),
        "docx" => extractor::extract_text_docx(file_path),
        "xlsx" | "xls" => extractor::extract_text_xlsx(file_path),
        _ => Err(format!("Unsupported format: {}", extension)),
    }
}
