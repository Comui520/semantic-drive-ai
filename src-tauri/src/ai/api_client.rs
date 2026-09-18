use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::collections::BTreeMap;

/// Configuration for an API endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpointConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub enabled: bool,
    pub timeout_secs: u64,
}

impl Default for ApiEndpointConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            enabled: false,
            timeout_secs: 30,
        }
    }
}

// ── Embedding API (SiliconFlow / OpenAI-compatible) ──

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
    encoding_format: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

/// Call SiliconFlow (or any OpenAI-compatible) embedding API.
/// Returns a list of embedding vectors, one per input text.
pub fn call_embedding_api(
    config: &ApiEndpointConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    if config.api_key.is_empty() {
        return Err("API key is not configured".to_string());
    }

    let url = format!("{}/embeddings", config.base_url.trim_end_matches('/'));
    let body = serde_json::to_string(&EmbeddingRequest {
        model: &config.model,
        input: texts,
        encoding_format: "float",
    })
    .map_err(|e| format!("Failed to serialize embedding request: {}", e))?;

    let response = ureq::post(&url)
        .header("Authorization", &format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| format_ureq_error(&e, "Embedding API"))?;
    let status = response.status().as_u16();
    let response_body = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read embedding response: {}", e))?;

    if status != 200 {
        return Err(format_api_error(status, &response_body));
    }

    let parsed: EmbeddingResponse = serde_json::from_str(&response_body)
        .map_err(|e| format!("Failed to parse embedding response: {}", e))?;

    let mut data = parsed.data;
    data.sort_by_key(|d| d.index);

    let embeddings: Vec<Vec<f32>> = data.into_iter().map(|d| d.embedding).collect();

    if embeddings.len() != texts.len() {
        return Err(format!(
            "Embedding count mismatch: expected {}, got {}",
            texts.len(),
            embeddings.len()
        ));
    }

    Ok(embeddings)
}

// ── Chat API (DeepSeek / OpenAI-compatible) ──

#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatApiMessage],
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<&'a [ChatApiTool]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<&'a str>,
}

fn deserialize_null_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// A single message in OpenAI chat format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatApiMessage {
    pub role: String,
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatApiTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub index: Option<usize>,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionResult {
    pub text: String,
    pub tool_calls: Vec<ChatToolCall>,
}

impl ChatApiTool {
    pub fn function(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self {
            kind: "function".to_string(),
            function: ChatToolFunction {
                name: name.to_string(),
                description: description.to_string(),
                parameters,
            },
        }
    }
}

impl ChatApiMessage {
    pub fn system(content: &str) -> Self {
        Self { role: "system".to_string(), content: content.to_string(), tool_calls: None }
    }
    pub fn user(content: &str) -> Self {
        Self { role: "user".to_string(), content: content.to_string(), tool_calls: None }
    }
    pub fn assistant(content: &str) -> Self {
        Self { role: "assistant".to_string(), content: content.to_string(), tool_calls: None }
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatApiMessage,
}

/// SSE streaming chunk.
#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    choices: Vec<ChatStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamChoice {
    delta: ChatStreamDelta,
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatToolCallFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

/// Call chat completion API (non-streaming). Returns full assistant text.
pub fn call_chat_completion_api(
    config: &ApiEndpointConfig,
    messages: &[ChatApiMessage],
    temperature: f64,
    max_tokens: usize,
) -> Result<String, String> {
    Ok(call_chat_completion_api_with_tools(config, messages, temperature, max_tokens, &[])?.text)
}

/// Call a chat completion endpoint and preserve structured tool calls.
pub fn call_chat_completion_api_with_tools(
    config: &ApiEndpointConfig,
    messages: &[ChatApiMessage],
    temperature: f64,
    max_tokens: usize,
    tools: &[ChatApiTool],
) -> Result<ChatCompletionResult, String> {
    if config.api_key.is_empty() {
        return Err("API key is not configured".to_string());
    }
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::to_string(&ChatCompletionRequest {
        model: &config.model,
        messages,
        stream: false,
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
        tools: if tools.is_empty() { None } else { Some(tools) },
        tool_choice: if tools.is_empty() { None } else { Some("auto") },
    }).map_err(|e| format!("Failed to serialize chat request: {}", e))?;
    let response = ureq::post(&url)
        .header("Authorization", &format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| format_ureq_error(&e, "Chat API"))?;
    let status = response.status().as_u16();
    let response_body = response.into_body().read_to_string()
        .map_err(|e| format!("Failed to read chat response: {}", e))?;
    if status != 200 { return Err(format_api_error(status, &response_body)); }
    let parsed: ChatCompletionResponse = serde_json::from_str(&response_body)
        .map_err(|e| format!("Failed to parse chat response: {}", e))?;
    let message = parsed.choices.first().map(|c| &c.message)
        .ok_or_else(|| "Chat API returned no choices".to_string())?;
    Ok(ChatCompletionResult {
        text: message.content.clone(),
        tool_calls: message.tool_calls.clone().unwrap_or_default(),
    })
}

/// Call chat completion API with SSE streaming.
/// `on_token` receives each content delta. Returns full accumulated text.
pub fn call_chat_completion_streaming_api(
    config: &ApiEndpointConfig,
    messages: &[ChatApiMessage],
    temperature: f64,
    max_tokens: usize,
    cancel_flag: Option<&std::sync::atomic::AtomicBool>,
    on_token: &mut dyn FnMut(String),
) -> Result<String, String> {
    Ok(call_chat_completion_streaming_api_with_tools(
        config, messages, temperature, max_tokens, &[], cancel_flag, on_token
    )?.text)
}

/// Streaming chat completion with native OpenAI-compatible tool calling.
/// Text deltas are emitted immediately; tool-call deltas are assembled safely
/// and returned when the stream finishes.
pub fn call_chat_completion_streaming_api_with_tools(
    config: &ApiEndpointConfig,
    messages: &[ChatApiMessage],
    temperature: f64,
    max_tokens: usize,
    tools: &[ChatApiTool],
    cancel_flag: Option<&std::sync::atomic::AtomicBool>,
    on_token: &mut dyn FnMut(String),
) -> Result<ChatCompletionResult, String> {
    use std::sync::atomic::Ordering;
    if config.api_key.is_empty() { return Err("API key is not configured".to_string()); }
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::to_string(&ChatCompletionRequest {
        model: &config.model,
        messages,
        stream: true,
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
        tools: if tools.is_empty() { None } else { Some(tools) },
        tool_choice: if tools.is_empty() { None } else { Some("auto") },
    }).map_err(|e| format!("Failed to serialize streaming request: {}", e))?;
    let response = ureq::post(&url)
        .header("Authorization", &format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .send(body.as_str())
        .map_err(|e| format_ureq_error(&e, "Streaming Chat API"))?;
    let status = response.status().as_u16();
    if status != 200 {
        let response_body = response.into_body().read_to_string().unwrap_or_default();
        return Err(format_api_error(status, &response_body));
    }
    let reader = BufReader::new(response.into_body().into_reader());
    let mut full_text = String::new();
    let mut calls: BTreeMap<usize, ChatToolCall> = BTreeMap::new();
    for line in reader.lines() {
        if cancel_flag.map(|flag| flag.load(Ordering::Relaxed)).unwrap_or(false) { break; }
        let line = line.map_err(|e| format!("Stream read error: {}", e))?;
        let line = line.trim().to_string();
        if line.is_empty() { continue; }
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" { break; }
            let Ok(chunk) = serde_json::from_str::<ChatStreamChunk>(data) else { continue; };
            for choice in &chunk.choices {
                if let Some(content) = &choice.delta.content {
                    full_text.push_str(content);
                    on_token(content.clone());
                }
                if let Some(deltas) = &choice.delta.tool_calls {
                    for delta in deltas {
                        let call = calls.entry(delta.index).or_insert_with(|| ChatToolCall {
                            id: String::new(), kind: "function".to_string(), index: Some(delta.index),
                            function: ChatToolCallFunction { name: String::new(), arguments: String::new() },
                        });
                        if let Some(id) = &delta.id { call.id.push_str(id); }
                        if let Some(function) = &delta.function {
                            if let Some(name) = &function.name { call.function.name.push_str(name); }
                            if let Some(arguments) = &function.arguments { call.function.arguments.push_str(arguments); }
                        }
                    }
                }
            }
        }
    }
    Ok(ChatCompletionResult { text: full_text, tool_calls: calls.into_values().collect() })
}

// ── Connection Tests ──

pub fn test_embedding_connection(config: &ApiEndpointConfig) -> Result<(), String> {
    let test_texts = vec!["test".to_string()];
    call_embedding_api(config, &test_texts).map(|_| ())
}

pub fn test_chat_connection(config: &ApiEndpointConfig) -> Result<(), String> {
    let test_messages = vec![ChatApiMessage::user("Hi")];
    call_chat_completion_api(config, &test_messages, 0.0, 10).map(|_| ())
}

// ── Helpers ──

fn format_ureq_error(e: &ureq::Error, context: &str) -> String {
    match e {
        ureq::Error::StatusCode(code) => {
            format!("{} returned HTTP {}", context, code)
        }
        ureq::Error::HostNotFound => {
            format!("{} request failed: host not found (DNS)", context)
        }
        ureq::Error::ConnectionFailed => {
            format!("{} request failed: connection refused", context)
        }
        ureq::Error::Timeout(_) => {
            format!("{} request timed out", context)
        }
        ureq::Error::Tls(msg) => {
            format!("{} TLS/SSL error: {}", context, msg)
        }
        ureq::Error::Io(e) => {
            format!("{} network I/O error: {}", context, e)
        }
        _ => format!("{} request failed: {:?}", context, e),
    }
}

fn format_api_error(status: u16, body: &str) -> String {
    if let Ok(err) = serde_json::from_str::<ApiErrorResponse>(body) {
        format!("API error ({}): {}", status, err.error.message)
    } else {
        let truncated: String = body.chars().take(500).collect();
        format!("API error ({}): {}", status, truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_endpoint_config_default() {
        let cfg = ApiEndpointConfig::default();
        assert!(cfg.base_url.is_empty());
        assert!(cfg.api_key.is_empty());
        assert!(!cfg.enabled);
        assert_eq!(cfg.timeout_secs, 30);
    }

    #[test]
    fn test_chat_api_message_constructors() {
        let sys = ChatApiMessage::system("test");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content, "test");

        let user = ChatApiMessage::user("hello");
        assert_eq!(user.role, "user");

        let asst = ChatApiMessage::assistant("hi");
        assert_eq!(asst.role, "assistant");
    }

    #[test]
    fn test_call_embedding_api_empty_input() {
        let cfg = ApiEndpointConfig::default();
        let result = call_embedding_api(&cfg, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_call_embedding_api_no_key() {
        let cfg = ApiEndpointConfig::default();
        let result = call_embedding_api(&cfg, &["test".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API key"));
    }

    #[test]
    fn test_call_chat_api_no_key() {
        let cfg = ApiEndpointConfig::default();
        let result = call_chat_completion_api(&cfg, &[ChatApiMessage::user("hi")], 0.0, 10);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API key"));
    }
}
