use serde::{Deserialize, Serialize};
use crate::ai::api_client::{ApiEndpointConfig, ChatApiMessage, ChatApiTool};
use crate::ai::search::SearchEngine;
use crate::store::config_store::AppConfig;
use crate::store::MetadataStore;

// ── File actions (AI-requested file operations) ──

/// A file operation that the LLM can request the UI to execute.
/// Serialized as `{"cmd":"rename_file","params":{"old_path":"...","new_name":"..."}}`
/// for easy embedding in LLM output as [{"cmd":"...","params":{...}}].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "params")]
pub enum FileAction {
    #[serde(rename = "rename_file")]
    RenameFile {
        old_path: String,
        new_name: String,
    },
    #[serde(rename = "delete_file")]
    DeleteFile {
        file_path: String,
    },
    #[serde(rename = "move_file")]
    MoveFile {
        source: String,
        destination: String,
    },
    #[serde(rename = "copy_file")]
    CopyFile {
        source: String,
        destination: String,
    },
    #[serde(rename = "import_file")]
    ImportFile {
        source: String,
        destination: String,
    },
    #[serde(rename = "vault_add_file")]
    VaultAddFile {
        file_path: String,
        #[serde(default)]
        password: String,
    },
    #[serde(rename = "set_file_tags")]
    SetFileTags {
        file_id: String,
        tags: Vec<String>,
    },

    // ── File-ID-based actions (preferred — eliminates path hallucination) ──
    #[serde(rename = "move_file_by_id")]
    MoveFileById {
        file_id: String,
        destination: String,
    },
    #[serde(rename = "copy_file_by_id")]
    CopyFileById {
        file_id: String,
        destination: String,
    },
    #[serde(rename = "delete_file_by_id")]
    DeleteFileById {
        file_id: String,
    },
    #[serde(rename = "rename_file_by_id")]
    RenameFileById {
        file_id: String,
        new_name: String,
    },
    #[serde(rename = "vault_add_file_by_id")]
    VaultAddFileById {
        file_id: String,
        #[serde(default)]
        password: String,
    },
    /// Agent suggests a search query (frontend-rendered as clickable chip)
    #[serde(rename = "search_files")]
    SearchFiles {
        query: String,
    },
    /// Import a file from within the scan root (by file_id, not path)
    #[serde(rename = "import_file_by_id")]
    ImportFileById {
        file_id: String,
        destination: String,
    },
    /// Open a file with the default application
    #[serde(rename = "open_file_by_id")]
    OpenFileById {
        file_id: String,
    },
    /// Open file location in file explorer
    #[serde(rename = "open_file_location_by_id")]
    OpenFileLocationById {
        file_id: String,
    },
    /// Append tags to a file (preserves existing tags)
    #[serde(rename = "add_file_tags")]
    AddFileTags {
        file_id: String,
        tags: Vec<String>,
    },
    /// Remove specific tags from a file
    #[serde(rename = "remove_file_tags")]
    RemoveFileTags {
        file_id: String,
        tags: Vec<String>,
    },
    /// Navigate to classification page
    #[serde(rename = "classify_files")]
    ClassifyFiles,
    /// Navigate to dedup page
    #[serde(rename = "find_duplicates")]
    FindDuplicates,
}

/// Find the closing brace for a JSON object while respecting quoted strings.
/// LLM action payloads often contain JSON-like characters in filenames or text;
/// a plain brace counter would truncate those payloads.
fn json_object_end(input: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if in_string {
            if escaped { escaped = false; }
            else if ch == '\\' { escaped = true; }
            else if ch == '"' { in_string = false; }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { return Some(index + 1); }
            }
            _ => {}
        }
    }
    None
}

/// Extract all `[ACTION:{...}]` markers from an API response.
pub fn parse_actions(text: &str) -> Vec<FileAction> {
    let marker = "[action:";
    let text_lower = text.to_lowercase();
    let mut actions = Vec::new();
    let mut search_from = 0;

    while let Some(marker_start) = text_lower[search_from..].find(marker) {
        let json_start = search_from + marker_start + marker.len();
        let remaining = &text[json_start..];
        if let Some(end) = json_object_end(remaining) {
            if let Ok(action) = serde_json::from_str::<FileAction>(&remaining[..end]) {
                actions.push(action);
            }
            search_from = json_start + end;
        } else {
            break;
        }
    }
    actions
}

/// Event payload emitted after LLM generation with parsed actions for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatActionsEvent {
    pub session_id: String,
    pub actions: Vec<FileAction>,
}

/// Strip all action markers from text, returning clean display text.
pub fn strip_action_markers(text: &str) -> String {
    let marker = "[action:";
    let text_lower = text.to_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut search_from = 0;

    while let Some(marker_start) = text_lower[search_from..].find(marker) {
        let abs_pos = search_from + marker_start;
        result.push_str(&text[search_from..abs_pos]);
        if abs_pos > 0 && text.as_bytes()[abs_pos - 1] == b'`' { result.pop(); }
        let json_start = abs_pos + marker.len();
        let remaining = &text[json_start..];
        let Some(end) = json_object_end(remaining) else { break; };
        search_from = json_start + end;
        if text[search_from..].starts_with(']') { search_from += 1; }
        if search_from < text.len() && text.as_bytes()[search_from] == b'`' { search_from += 1; }
    }
    result.push_str(&text[search_from..]);
    result
}

/// Check whether LLM output contains any action markers.
pub fn has_actions(text: &str) -> bool {
    text.to_lowercase().contains("[action:")
}

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

    // File action intent — skip RAG, just let the LLM handle it
    let action_patterns = [
        "移动", "重命名", "删除", "复制", "移到", "放到", "搬到",
        "改名", "删掉", "复制到", "拷贝", "导入", "加密", "添加到安全空间",
        "设置标签", "打标签",
    ];
    if action_patterns.iter().any(|p| msg.contains(p)) {
        return ChatIntent::GeneralChat;
    }

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
    // But only if not already classified as an action above
    let file_keywords = ["文件", "文档", "文件夹", "目录", "内容", "我记得"];
    if file_keywords.iter().any(|k| msg.contains(k)) {
        return ChatIntent::SearchFiles;
    }

    ChatIntent::GeneralChat
}

// ── Chat prompt templates ──

const SYSTEM_PROMPT: &str = "你是 Semantic Drive AI（语义智能文件管家），运行在用户设备上的跨平台 AI 文件管理 Agent。\
你不仅可以聊天，更能**实际操作文件**——搜索、移动、重命名、打标签、加密、分类、去重。\
中文回答，简洁准确。先给出简短计划；涉及文件变更时只生成带 file_id 的待确认操作，绝不假定操作已经执行。

## 你可以做的事（操作列表）

优先使用 API 提供的原生 JSON Schema tools 来请求操作；对于不支持 tool calling 的服务，再在回复中嵌入 [ACTION:JSON] 标记。用户确认后自动执行。\
**必须用 file_id，禁止用路径字符串。** 支持的操作：

**文件操作：**
- 移动 [ACTION:{\"cmd\":\"move_file_by_id\",\"params\":{\"file_id\":\"xxx\",\"destination\":\"目标文件夹/\"}}]
- 复制 [ACTION:{\"cmd\":\"copy_file_by_id\",\"params\":{\"file_id\":\"xxx\",\"destination\":\"目标文件夹/\"}}]
- 重命名 [ACTION:{\"cmd\":\"rename_file_by_id\",\"params\":{\"file_id\":\"xxx\",\"new_name\":\"新名称.txt\"}}]
- 删除 [ACTION:{\"cmd\":\"delete_file_by_id\",\"params\":{\"file_id\":\"xxx\"}}]
- 打开 [ACTION:{\"cmd\":\"open_file_by_id\",\"params\":{\"file_id\":\"xxx\"}}]
- 打开位置 [ACTION:{\"cmd\":\"open_file_location_by_id\",\"params\":{\"file_id\":\"xxx\"}}]
- 导入 [ACTION:{\"cmd\":\"import_file_by_id\",\"params\":{\"file_id\":\"xxx\",\"destination\":\"目标文件夹/\"}}]

**标签管理：**
- 设置标签 [ACTION:{\"cmd\":\"set_file_tags\",\"params\":{\"file_id\":\"xxx\",\"tags\":[\"标签\"]}}] ← 替换全部标签
- 添加标签 [ACTION:{\"cmd\":\"add_file_tags\",\"params\":{\"file_id\":\"xxx\",\"tags\":[\"新标签\"]}}] ← 追加不覆盖
- 移除标签 [ACTION:{\"cmd\":\"remove_file_tags\",\"params\":{\"file_id\":\"xxx\",\"tags\":[\"要删的标签\"]}}]

**安全空间：**
- 加密 [ACTION:{\"cmd\":\"vault_add_file_by_id\",\"params\":{\"file_id\":\"xxx\"}}] ← 需用户在安全空间页面先解锁

**页面跳转：**
- 搜索 [ACTION:{\"cmd\":\"search_files\",\"params\":{\"query\":\"搜索词\"}}]
- 分类 [ACTION:{\"cmd\":\"classify_files\"}]
- 去重 [ACTION:{\"cmd\":\"find_duplicates\"}]

## 重要规则
1. 只用 file_id，不用路径。一个标记一个操作。标记嵌入正文，不用代码块。
2. set_file_tags 替换全部标签；add_file_tags 追加；remove_file_tags 移除指定标签。区分清楚！
3. 删除、移动等风险操作仅用户明确要求时使用。
4. 文件夹路径直接作为 destination，不加额外子目录。
5. 批量操作允许多个 action 标记同时出现。
6. 你不会知道用户的加密空间密码——如果用户要求加密，发 vault_add 操作，系统用已解锁的密码。

## 行为风格
- 搜索文件后：列出 3-5 个最相关文件 + 主动建议下一步（分类/去重/标签/加密）
- 低匹配度结果直接忽略，别提它们。搜索无结果诚实告知。
- 发现文件混乱主动建议分类；发现重复指出浪费空间。
- 绝不编造文件或内容。所有信息来自实际数据。";

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
    let max_history = 8; // 4 exchanges × 2 messages each
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
            "以下是搜索到的文件（按相关度排序，低相关度结果已过滤）：\n{}\n\n用户的问题是：{}",
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
    let content_scores = store.search_content_fts5(query).unwrap_or_default();
    let content_score_map: std::collections::HashMap<String, f32> = content_scores.into_iter().collect();
    // Use a larger max_results for the engine, then filter by score threshold afterward
    let results = search_engine.search(query, &files, (max_results * 2).max(20), bilingual, Some(&content_score_map));

    // Score threshold: only include results with meaningful relevance
    const MIN_SCORE: f32 = 0.15;
    let relevant: Vec<_> = results.into_iter().filter(|r| r.score >= MIN_SCORE).collect();

    if relevant.is_empty() {
        return Ok(("未找到匹配的文件。请尝试更具体的关键词。".to_string(), Vec::new()));
    }

    let mut ctx = String::new();
    let mut file_refs = Vec::new();
    for (i, r) in relevant.iter().take(max_results).enumerate() {
        ctx.push_str(&format!(
            "{}. **{}** (ID: `{}`, 路径: `{}`)\n   类型: {} | 匹配度: {:.0}%",
            i + 1, r.file_name, r.file_id, r.file_path, r.match_type, r.score * 100.0
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

// ── API Mode Helpers ──

/// Build OpenAI-compatible messages array for API calls.
/// Returns system prompt first, then conversation history, then user message with RAG context.
pub fn build_chat_messages_api(
    history: &[ChatMessage],
    user_message: &str,
    rag_context: Option<&str>,
    file_context: Option<&str>,
) -> Vec<ChatApiMessage> {
    let mut messages: Vec<ChatApiMessage> = Vec::new();

    // System message with file context if available
    let mut system_content = SYSTEM_PROMPT.to_string();
    if let Some(fc) = file_context {
        system_content.push_str("\n\n--- 当前文件上下文 ---\n");
        system_content.push_str(fc);
    }
    messages.push(ChatApiMessage::system(&system_content));

    // Conversation history (limit to last 20 messages for API token limits)
    let skip = if history.len() > 20 { history.len() - 20 } else { 0 };
    for msg in history.iter().skip(skip) {
        let role = if msg.role == "user" { "user" } else { "assistant" };
        messages.push(ChatApiMessage {
            role: role.to_string(),
            content: msg.content.clone(),
            tool_calls: None,
        });
    }

    // Current user message with optional RAG context
    let mut user_content = user_message.to_string();
    if let Some(rag) = rag_context {
        user_content.push_str("\n\n--- 相关文件信息 ---\n");
        user_content.push_str(rag);
    }
    messages.push(ChatApiMessage::user(&user_content));

    messages
}

/// Determine whether to use API mode for chat, based on config.
pub fn resolve_chat_mode(config: &AppConfig) -> (bool, ApiEndpointConfig) {
    let ep = ApiEndpointConfig {
        base_url: config.chat_api.base_url.clone(),
        api_key: config.chat_api.api_key.clone(),
        model: config.chat_api.model.clone(),
        enabled: config.chat_api.enabled && !config.chat_api.api_key.trim().is_empty(),
        timeout_secs: config.chat_api.timeout_secs,
    };
    (ep.enabled, ep)
}

/// Determine whether to use API mode for embedding, based on config.
pub fn resolve_embedding_mode(config: &AppConfig) -> (bool, ApiEndpointConfig) {
    let ep = ApiEndpointConfig {
        base_url: config.embedding_api.base_url.clone(),
        api_key: config.embedding_api.api_key.clone(),
        model: config.embedding_api.model.clone(),
        enabled: config.embedding_api.enabled && !config.embedding_api.api_key.trim().is_empty(),
        timeout_secs: config.embedding_api.timeout_secs,
    };
    (ep.enabled, ep)
}

/// OpenAI-compatible tools exposed to the Agent. The UI still supports the
/// legacy `[ACTION:...]` protocol for providers that do not implement tools.
pub fn action_tools() -> Vec<ChatApiTool> {
    let object = |properties: serde_json::Value, required: &[&str]| {
        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        })
    };
    vec![
        ChatApiTool::function("move_file_by_id", "Move an indexed file into a workspace-relative folder.", object(serde_json::json!({
            "file_id": {"type":"string"}, "destination": {"type":"string"}
        }), &["file_id", "destination"])),
        ChatApiTool::function("copy_file_by_id", "Copy an indexed file into a workspace-relative folder.", object(serde_json::json!({
            "file_id": {"type":"string"}, "destination": {"type":"string"}
        }), &["file_id", "destination"])),
        ChatApiTool::function("rename_file_by_id", "Rename an indexed file without changing its folder.", object(serde_json::json!({
            "file_id": {"type":"string"}, "new_name": {"type":"string"}
        }), &["file_id", "new_name"])),
        ChatApiTool::function("delete_file_by_id", "Delete an indexed file after explicit user confirmation.", object(serde_json::json!({
            "file_id": {"type":"string"}
        }), &["file_id"])),
        ChatApiTool::function("set_file_tags", "Replace the custom tags of an indexed file.", object(serde_json::json!({
            "file_id": {"type":"string"}, "tags": {"type":"array", "items":{"type":"string"}}
        }), &["file_id", "tags"])),
        ChatApiTool::function("add_file_tags", "Append custom tags to an indexed file.", object(serde_json::json!({
            "file_id": {"type":"string"}, "tags": {"type":"array", "items":{"type":"string"}}
        }), &["file_id", "tags"])),
        ChatApiTool::function("remove_file_tags", "Remove custom tags from an indexed file.", object(serde_json::json!({
            "file_id": {"type":"string"}, "tags": {"type":"array", "items":{"type":"string"}}
        }), &["file_id", "tags"])),
        ChatApiTool::function("open_file_by_id", "Open a file with the operating system default application.", object(serde_json::json!({
            "file_id": {"type":"string"}
        }), &["file_id"])),
        ChatApiTool::function("open_file_location_by_id", "Reveal an indexed file in the file manager.", object(serde_json::json!({
            "file_id": {"type":"string"}
        }), &["file_id"])),
        ChatApiTool::function("search_files", "Search files in the current workspace.", object(serde_json::json!({
            "query": {"type":"string"}
        }), &["query"])),
        ChatApiTool::function("classify_files", "Open the classification view.", object(serde_json::json!({"type":"object", "properties":{}, "additionalProperties":false}), &[])),
        ChatApiTool::function("find_duplicates", "Open duplicate detection.", object(serde_json::json!({"type":"object", "properties":{}, "additionalProperties":false}), &[])),
    ]
}

/// Convert a native tool call into the action envelope consumed by the
/// confirmation UI and Rust validator.
pub fn tool_calls_to_actions(calls: &[crate::ai::api_client::ChatToolCall]) -> Vec<FileAction> {
    calls.iter().filter_map(|call| {
        let args: serde_json::Value = serde_json::from_str(&call.function.arguments).ok()?;
        let mut envelope = serde_json::Map::new();
        envelope.insert("cmd".into(), serde_json::Value::String(call.function.name.clone()));
        envelope.insert("params".into(), args);
        serde_json::from_value(serde_json::Value::Object(envelope)).ok()
    }).collect()
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_actions ──

    #[test]
    fn test_parse_rename_action() {
        let text = r#"我来帮你重命名这个文件。[ACTION:{"cmd":"rename_file","params":{"old_path":"docs/report.docx","new_name":"final.docx"}}]"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FileAction::RenameFile { old_path, new_name } => {
                assert_eq!(old_path, "docs/report.docx");
                assert_eq!(new_name, "final.docx");
            }
            other => panic!("Expected RenameFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_delete_action() {
        let text = r#"已为您删除。[ACTION:{"cmd":"delete_file","params":{"file_path":"temp/old.txt"}}]"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FileAction::DeleteFile { file_path } => {
                assert_eq!(file_path, "temp/old.txt");
            }
            other => panic!("Expected DeleteFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_move_action() {
        let text = r#"[ACTION:{"cmd":"move_file","params":{"source":"docs/a.pdf","destination":"archive/a.pdf"}}]"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FileAction::MoveFile { source, destination } => {
                assert_eq!(source, "docs/a.pdf");
                assert_eq!(destination, "archive/a.pdf");
            }
            other => panic!("Expected MoveFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_copy_action() {
        let text = r#"[ACTION:{"cmd":"copy_file","params":{"source":"a.txt","destination":"backup/a.txt"}}] 已复制。"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FileAction::CopyFile { source, destination } => {
                assert_eq!(source, "a.txt");
                assert_eq!(destination, "backup/a.txt");
            }
            other => panic!("Expected CopyFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_set_tags_action() {
        let text = r#"[ACTION:{"cmd":"set_file_tags","params":{"file_id":"abc123","tags":["重要","合同"]}}]"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FileAction::SetFileTags { file_id, tags } => {
                assert_eq!(file_id, "abc123");
                assert_eq!(tags, &vec!["重要".to_string(), "合同".to_string()]);
            }
            other => panic!("Expected SetFileTags, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_import_action() {
        let text = r#"已导入。[ACTION:{"cmd":"import_file","params":{"source":"C:/Downloads/data.xlsx","destination":"invoices/data.xlsx"}}]"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FileAction::ImportFile { source, destination } => {
                assert_eq!(source, "C:/Downloads/data.xlsx");
                assert_eq!(destination, "invoices/data.xlsx");
            }
            other => panic!("Expected ImportFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_multiple_actions() {
        let text = r#"执行以下操作：
[ACTION:{"cmd":"rename_file","params":{"old_path":"a.txt","new_name":"b.txt"}}]
[ACTION:{"cmd":"delete_file","params":{"file_path":"c.txt"}}]
完成。"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], FileAction::RenameFile { .. }));
        assert!(matches!(actions[1], FileAction::DeleteFile { .. }));
    }

    #[test]
    fn test_parse_no_actions() {
        assert!(parse_actions("你好，有什么可以帮你的？").is_empty());
        assert!(parse_actions("").is_empty());
        assert!(parse_actions("[ACTION:invalid json here]").is_empty());
    }

    #[test]
    fn test_parse_malformed_marker() {
        // Missing closing brace — should skip gracefully
        let text = r#"[ACTION:{"cmd":"rename_file","params":{"old_path":"a.txt"}} 没有闭合"#;
        let actions = parse_actions(text);
        assert!(actions.is_empty());
    }

    // ── strip_action_markers ──

    #[test]
    fn test_strip_single_marker() {
        let text = r#"已重命名。[ACTION:{"cmd":"rename_file","params":{"old_path":"a.txt","new_name":"b.txt"}}]"#;
        let result = strip_action_markers(text);
        assert_eq!(result, "已重命名。");
    }

    #[test]
    fn test_strip_multiple_markers() {
        let text = r#"操作1。[ACTION:{"cmd":"rename_file","params":{"old_path":"a.txt","new_name":"b.txt"}}]
操作2。[ACTION:{"cmd":"delete_file","params":{"file_path":"c.txt"}}]
完成。"#;
        let result = strip_action_markers(text);
        assert_eq!(result, "操作1。\n操作2。\n完成。");
    }

    #[test]
    fn test_action_parser_ignores_braces_inside_strings() {
        let text = r#"请处理该文件。[ACTION:{"cmd":"rename_file_by_id","params":{"file_id":"abc","new_name":"report {final}.txt"}}]"#;
        let actions = parse_actions(text);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], FileAction::RenameFileById { new_name, .. } if new_name == "report {final}.txt"));
    }

    #[test]
    fn test_strip_no_markers() {
        let text = "正常文本，没有操作标记。";
        assert_eq!(strip_action_markers(text), text);
    }

    #[test]
    fn test_strip_empty_text() {
        assert_eq!(strip_action_markers(""), "");
    }

    #[test]
    fn test_strip_marker_at_start() {
        let text = r#"[ACTION:{"cmd":"delete_file","params":{"file_path":"a.txt"}}]已删除。"#;
        assert_eq!(strip_action_markers(text), "已删除。");
    }

    #[test]
    fn test_strip_marker_only() {
        let text = r#"前面[ACTION:{"cmd":"rename_file","params":{"old_path":"a.txt","new_name":"b.txt"}}]中间[ACTION:{"cmd":"delete_file","params":{"file_path":"c.txt"}}]后面"#;
        assert_eq!(strip_action_markers(text), "前面中间后面");
    }

    #[test]
    fn test_strip_marker_inside_text() {
        let text = r#"我把文件[ACTION:{"cmd":"rename_file","params":{"old_path":"a.txt","new_name":"b.txt"}}]重命名了。"#;
        assert_eq!(strip_action_markers(text), "我把文件重命名了。");
    }

    // ── has_actions ──

    #[test]
    fn test_has_actions_true() {
        assert!(has_actions(r#"text [ACTION:{"cmd":"delete_file","params":{"file_path":"a.txt"}}] more"#));
    }

    #[test]
    fn test_has_actions_false() {
        assert!(!has_actions("普通对话文本"));
        assert!(!has_actions(""));
        assert!(!has_actions("[ACTION:"));
    }

    // ── FileAction serialization round-trip ──

    #[test]
    fn test_serialize_roundtrip_rename() {
        let action = FileAction::RenameFile {
            old_path: "a.txt".into(),
            new_name: "b.txt".into(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, r#"{"cmd":"rename_file","params":{"old_path":"a.txt","new_name":"b.txt"}}"#);
        let deserialized: FileAction = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, FileAction::RenameFile { .. }));
    }

    #[test]
    fn test_serialize_roundtrip_set_tags() {
        let action = FileAction::SetFileTags {
            file_id: "id1".into(),
            tags: vec!["tag1".into(), "tag2".into()],
        };
        let json = serde_json::to_string(&action).unwrap();
        let deserialized: FileAction = serde_json::from_str(&json).unwrap();
        match deserialized {
            FileAction::SetFileTags { file_id, tags } => {
                assert_eq!(file_id, "id1");
                assert_eq!(tags, vec!["tag1", "tag2"]);
            }
            other => panic!("Expected SetFileTags, got {:?}", other),
        }
    }

    #[test]
    fn test_native_tool_schema_contains_file_id_boundary() {
        let tools = action_tools();
        let rename = tools.iter().find(|tool| tool.function.name == "rename_file_by_id").unwrap();
        assert_eq!(rename.kind, "function");
        assert!(rename.function.parameters["required"].as_array().unwrap().iter().any(|v| v == "file_id"));
    }

    #[test]
    fn test_native_tool_call_is_normalized_to_action() {
        let call = crate::ai::api_client::ChatToolCall {
            id: "call-1".into(), kind: "function".into(), index: Some(0),
            function: crate::ai::api_client::ChatToolCallFunction {
                name: "rename_file_by_id".into(),
                arguments: r#"{"file_id":"abc","new_name":"new.txt"}"#.into(),
            },
        };
        let actions = tool_calls_to_actions(&[call]);
        assert!(matches!(&actions[0], FileAction::RenameFileById { file_id, new_name } if file_id == "abc" && new_name == "new.txt"));
    }

    #[test]
    fn test_deserialize_action_from_marker() {
        // Simulate exactly what the LLM outputs inside [ACTION:...]
        let json = r#"{"cmd":"move_file","params":{"source":"src/a.rs","destination":"src/b.rs"}}"#;
        let action: FileAction = serde_json::from_str(json).unwrap();
        match action {
            FileAction::MoveFile { source, destination } => {
                assert_eq!(source, "src/a.rs");
                assert_eq!(destination, "src/b.rs");
            }
            other => panic!("Expected MoveFile, got {:?}", other),
        }
    }
}
