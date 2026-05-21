use serde::{Deserialize, Serialize};
use crate::ai::search::SearchEngine;
use crate::store::MetadataStore;

// ── File actions (AI-requested file operations) ──

/// A file operation that the LLM can request the UI to execute.
/// Serialized as `{"cmd":"rename_file","params":{"old_path":"...","new_name":"..."}}`
/// for easy embedding in LLM output as `[ACTION:{"cmd":"...","params":{...}}]`.
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
        password: String,
    },
}

/// Extract all `[ACTION:{"cmd":"...","params":{...}}]` markers from LLM output text.
/// Uses brace-depth counting to correctly handle nested JSON objects.
pub fn parse_actions(text: &str) -> Vec<FileAction> {
    let marker = "[ACTION:";
    let mut actions = Vec::new();
    let mut search_from = 0;

    while let Some(marker_start) = text[search_from..].find(marker) {
        let json_start = search_from + marker_start + marker.len();
        let remaining = &text[json_start..];

        let mut brace_depth: i32 = 0;
        let mut json_end = None;
        for (i, c) in remaining.char_indices() {
            match c {
                '{' => brace_depth += 1,
                '}' => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        json_end = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(end) = json_end {
            let json_str = &remaining[..end];
            if let Ok(action) = serde_json::from_str::<FileAction>(json_str) {
                actions.push(action);
            }
            search_from = json_start + end;
        } else {
            search_from = json_start + 1;
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

/// Strip all `[ACTION:{...}]` markers from text, returning clean display text.
pub fn strip_action_markers(text: &str) -> String {
    let marker = "[ACTION:";
    let mut result = String::with_capacity(text.len());
    let mut search_from = 0;

    while let Some(marker_start) = text[search_from..].find(marker) {
        result.push_str(&text[search_from..search_from + marker_start]);
        let json_start = search_from + marker_start + marker.len();
        let remaining = &text[json_start..];

        let mut brace_depth: i32 = 0;
        let mut json_end = None;
        for (i, c) in remaining.char_indices() {
            match c {
                '{' => brace_depth += 1,
                '}' => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        json_end = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(end) = json_end {
            search_from = json_start + end;
        } else {
            search_from = json_start + 1;
        }
    }

    result.push_str(&text[search_from..]);
    result
}

/// Check whether LLM output contains any action markers.
pub fn has_actions(text: &str) -> bool {
    text.contains("[ACTION:")
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

## 文件操作能力

**你不仅能回答问题，还能直接执行文件操作！** 在回复中嵌入 `[ACTION:JSON]` 标记即可请求执行操作。
标记会被自动提取执行，用户会看到确认提示。支持的操作为：

### 重命名文件
`[ACTION:{\"cmd\":\"rename_file\",\"params\":{\"old_path\":\"相对路径/旧文件名.txt\",\"new_name\":\"新文件名.txt\"}}]`

### 删除文件
`[ACTION:{\"cmd\":\"delete_file\",\"params\":{\"file_path\":\"相对路径/文件名.txt\"}}]`

### 移动文件
`[ACTION:{\"cmd\":\"move_file\",\"params\":{\"source\":\"相对路径/源文件.txt\",\"destination\":\"目标文件夹/源文件.txt\"}}]`

### 复制文件
`[ACTION:{\"cmd\":\"copy_file\",\"params\":{\"source\":\"相对路径/源文件.txt\",\"destination\":\"目标文件夹/源文件.txt\"}}]`

### 导入外部文件
`[ACTION:{\"cmd\":\"import_file\",\"params\":{\"source\":\"C:/外部文件.txt\",\"destination\":\"目标文件夹/文件名.txt\"}}]`

### 添加标签
`[ACTION:{\"cmd\":\"set_file_tags\",\"params\":{\"file_id\":\"文件ID\",\"tags\":[\"标签1\",\"标签2\"]}}]`

**使用规则：**
1. **优先使用 file_id-based 操作**（推荐，路径更准确）：
   - 当看到文件列表中的 `ID: xxxxx` 时，在 action 中使用 `file_id` 参数
   - 例：`[ACTION:{\"cmd\":\"move_file_by_id\",\"params\":{\"file_id\":\"xxx-xxx\",\"destination\":\"目标文件夹/\"}}]`
   - 例：`[ACTION:{\"cmd\":\"rename_file_by_id\",\"params\":{\"file_id\":\"xxx-xxx\",\"new_name\":\"新名称.txt\"}}]`
   - 例：`[ACTION:{\"cmd\":\"delete_file_by_id\",\"params\":{\"file_id\":\"xxx-xxx\"}}]`
   - `copy_file_by_id` 和 `vault_add_file_by_id` 也类似
2. 只有当没有 file_id 可用时（如用户手动输入路径），才使用旧的路径-based action。
3. 每个 `[ACTION:...]` 只能对应一个操作。需要多个操作时，输出多个标记。
4. 操作标记应嵌入在回复文本中合适的位置，用户在确认前会看到你的完整回复。
5. 用户的文件路径都是相对于应用扫描根目录的相对路径（如 `文档/报告.pdf`）。

**路径格式严格要求（非常重要！）：**
- 禁止添加前导斜杠 `/`。正确：`文档/报告.pdf`，错误：`/文档/报告.pdf`
- 禁止 URL 编码。正确：`合同 2024.pdf`，错误：`合同%202024.pdf`
- 禁止改变文件扩展名。原文件是 `.md` 就写 `.md`，不能改成 `.pdf`
- 禁止在路径前后添加多余空格
- 目标路径也必须是相对路径，文件夹名不能带多余空格
- 仔细查看文件上下文中的\"附加文件路径列表\"，原样使用那里的路径
- 禁止将简体中文转换为繁体中文。如果路径中是\"复杂度\"，就写\"复杂度\"，不要写\"複雜度\"

6. 只有用户明确要求执行操作时，才使用 action 标记。不要自作主张。
7. 删除/移动等有风险的操作，一定要用户明确表达意图后才使用 action 标记。
8. 操作执行后会自动显示结果，回复中无需重复说明操作已执行。
9. 如果操作需要信息（如密码、目标路径），先问清楚再使用 action 标记。
10. 操作执行后建议告知用户结果，或提出下一步建议。

## 助手行为指南

1. 当用户问「重复文件」「清理空间」「释放空间」 → 建议打开**整理建议**页面查看重复文件和大文件
2. 当用户希望保护隐私文件 → 建议使用**安全空间**进行加密存储
3. 当用户想重命名/移动/复制/删除文件 → **使用 action 标记直接执行**，不需要让用户手动操作
4. 当用户想知道文件分类 → 建议打开**文件分类**页面查看
5. 当用户想搜索文件 → 使用 RAG 搜索功能，返回匹配的文件信息
6. 附加文件时：分析文件内容，给出针对性的操作建议，必要时使用 action 标记执行
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
