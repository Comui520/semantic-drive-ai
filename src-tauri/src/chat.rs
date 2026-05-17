use serde::{Deserialize, Serialize};
use crate::ai::search::SearchEngine;
use crate::store::MetadataStore;

// ── Data structures ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub file_refs: Option<Vec<FileRef>>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    pub file_id: String,
    pub file_name: String,
    pub file_path: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTokenEvent {
    pub session_id: String,
    pub token: String,
    pub done: bool,
}

// ── Intent detection ──

#[derive(Debug, Clone, PartialEq)]
pub enum ChatIntent {
    GeneralChat,
    SearchFiles,
    SummarizeFile,
    Unknown,
}

/// Detect user intent from message text using rule-based keyword matching.
pub fn detect_intent(message: &str) -> ChatIntent {
    let msg = message.trim();

    // Search intent
    let search_patterns = ["找", "搜索", "查", "搜一下", "帮我找", "找一下", "有没有", "查找", "寻找"];
    if search_patterns.iter().any(|p| msg.contains(p)) {
        return ChatIntent::SearchFiles;
    }

    // Summarize intent
    let summarize_patterns = ["总结", "摘要", "概括", "归纳"];
    if summarize_patterns.iter().any(|p| msg.contains(p)) {
        return ChatIntent::SummarizeFile;
    }

    // If message contains file-related keywords without clear intent, still search
    let file_keywords = ["文件", "文档", "文件夹", "目录", "内容", "我记得"];
    if file_keywords.iter().any(|k| msg.contains(k)) {
        return ChatIntent::SearchFiles;
    }

    ChatIntent::GeneralChat
}

// ── Chat prompt templates ──

const SYSTEM_PROMPT: &str = "你是 Semantic Drive AI（语义智能文件管家）的智能助手，运行在用户的U盘/移动硬盘上。\
你的任务是帮助用户管理、查找、整理文件，并回答相关问题。\
请用中文回答，简洁准确，必要时给出具体操作建议。

## 应用功能概览

### 1. 智能搜索 (SmartSearch)
- 支持自然语言搜索文件，例如「上周的Excel」「2024年的照片」
- 支持按自定义标签搜索和浏览
- 支持目录浏览、文件操作（重命名、移动、复制、删除）
- 搜索结果按文件夹分组展示，可点击打开文件或定位文件位置

### 2. 文件分类 (FileClassify)
- 自动按文件类型和内容将文件归类（文档、图片、视频、代码等）
- 智能标签：自动提取关键词标签
- 点击分类可查看该类下的所有文件，并可在搜索中定位

### 3. 整理建议 (OrganizeSuggestions)
- 检测重复文件（基于BLAKE3哈希），显示可清理的空间
- 显示大文件列表，帮助清理磁盘空间
- 当用户询问「哪些文件重复」「如何清理空间」时，建议打开整理建议页面

### 4. 安全空间 (SecureSpace)
- AES-256-GCM + Argon2id 加密存储
- 保护隐私文件，需要密码才能访问
- 当用户想保护敏感文件时，建议使用安全空间

### 5. 已支持的Tauri命令（部分）：
- 文件操作：open_file, open_file_location, rename_file, delete_file, copy_file, move_file, import_file
- 标签管理：set_file_custom_tags, get_all_tags_with_counts, get_files_by_custom_tag
- 分类：classify_files, get_files_by_category
- 去重：find_duplicates, organizededuplicates
- 安全空间：vault_list, vault_encrypt, vault_decrypt, vault_delete
- 搜索：search_files, cancel_search

## 助手行为指南

1. 当用户问「重复文件」「清理空间」「释放空间」 → 建议打开**整理建议**页面查看重复文件和大文件
2. 当用户希望保护隐私文件 → 建议使用**安全空间**进行加密存储
3. 当用户想重命名/移动/复制文件 → 说明可在文件浏览器中操作（右键点击文件）
4. 当用户想知道文件分类 → 建议打开**文件分类**页面查看
5. 当用户想搜索文件 → 使用 RAG 搜索功能，返回匹配的文件信息
6. 附加文件时：分析文件内容，给出针对性的操作建议
7. 绝对诚实：不编造不存在的信息。如果不清楚某个功能，如实告知
8. 如果用户问的是纯知识性问题，直接回答，无需涉及文件功能

请始终以有帮助的、专业的助手身份回应。";

const CHAT_TEMPLATE_SYSTEM: &str = "<|im_start|>system\n{SYSTEM}<|im_end|>\n";
const CHAT_TEMPLATE_USER: &str = "<|im_start|>user\n{CONTENT}<|im_end|>\n";
const CHAT_TEMPLATE_ASSISTANT: &str = "<|im_start|>assistant\n{CONTENT}<|im_end|>\n";
const CHAT_TEMPLATE_START: &str = "<|im_start|>assistant\n";

/// Build a chat prompt from system prompt, history, user message, file context, and RAG context.
pub fn build_chat_prompt(
    history: &[ChatMessage],
    user_message: &str,
    rag_context: Option<&str>,
    file_context: Option<&str>,
) -> String {
    let mut prompt = CHAT_TEMPLATE_SYSTEM.replace("{SYSTEM}", SYSTEM_PROMPT);

    // Add history (last 6 exchanges for context window management)
    let max_history = 12; // 6 exchanges × 2 messages each
    let start = history.len().saturating_sub(max_history);
    for msg in &history[start..] {
        match msg.role.as_str() {
            "user" => prompt.push_str(&CHAT_TEMPLATE_USER.replace("{CONTENT}", &msg.content)),
            "assistant" => prompt.push_str(&CHAT_TEMPLATE_ASSISTANT.replace("{CONTENT}", &msg.content)),
            _ => {}
        }
    }

    // Build final user message content with optional file context and RAG context
    let final_content = match (file_context, rag_context) {
        (Some(fc), Some(rc)) => format!(
            "以下是附加的文件内容：\n{}\n\n以下是根据搜索找到的相关文件信息：\n{}\n\n用户的问题是：{}",
            fc, rc, user_message
        ),
        (Some(fc), None) => format!(
            "以下是附加的文件内容：\n{}\n\n用户的问题是：{}",
            fc, user_message
        ),
        (None, Some(rc)) => format!(
            "以下是搜索到的相关文件信息：\n{}\n\n用户的问题是：{}",
            rc, user_message
        ),
        (None, None) => user_message.to_string(),
    };

    prompt.push_str(&CHAT_TEMPLATE_USER.replace("{CONTENT}", &final_content));

    prompt.push_str(CHAT_TEMPLATE_START);
    prompt
}

/// Perform a search and return formatted results as RAG context text + structured FileRefs.
pub fn search_to_rag_context(
    search_engine: &SearchEngine,
    store: &MetadataStore,
    query: &str,
    max_results: usize,
    bilingual: bool,
) -> Result<(String, Vec<FileRef>), String> {
    let files = store.get_all_files()?;
    let results = search_engine.search(query, &files, max_results, bilingual);

    if results.is_empty() {
        return Ok(("未找到匹配的文件。".to_string(), Vec::new()));
    }

    let mut ctx = String::new();
    let mut file_refs = Vec::new();
    for (i, r) in results.iter().enumerate() {
        ctx.push_str(&format!(
            "{}. **{}** (路径: `{}`)\n   类型: {} | 匹配度: {:.0}%",
            i + 1, r.file_name, r.file_path, r.match_type, r.score * 100.0
        ));
        if !r.snippet.is_empty() {
            let snippet: String = r.snippet.chars().take(300).collect();
            ctx.push_str(&format!("\n   内容预览: {}", snippet));
        }
        ctx.push('\n');
        file_refs.push(FileRef {
            file_id: r.file_id.clone(),
            file_name: r.file_name.clone(),
            file_path: r.file_path.clone(),
            snippet: r.snippet.chars().take(300).collect(),
        });
    }
    Ok((ctx, file_refs))
}
