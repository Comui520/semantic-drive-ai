use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};

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
}

/// A single message in OpenAI chat format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatApiMessage {
    pub role: String,
    pub content: String,
}

impl ChatApiMessage {
    pub fn system(content: &str) -> Self {
        Self { role: "system".to_string(), content: content.to_string() }
    }
    pub fn user(content: &str) -> Self {
        Self { role: "user".to_string(), content: content.to_string() }
    }
    pub fn assistant(content: &str) -> Self {
        Self { role: "assistant".to_string(), content: content.to_string() }
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
    })
    .map_err(|e| format!("Failed to serialize chat request: {}", e))?;

    let response = ureq::post(&url)
        .header("Authorization", &format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .send(body.as_str())
        .map_err(|e| format_ureq_error(&e, "Chat API"))?;

    let status = response.status().as_u16();
    let response_body = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read chat response: {}", e))?;

    if status != 200 {
        return Err(format_api_error(status, &response_body));
    }

    let parsed: ChatCompletionResponse = serde_json::from_str(&response_body)
        .map_err(|e| format!("Failed to parse chat response: {}", e))?;

    parsed.choices.first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| "Chat API returned no choices".to_string())
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
    use std::sync::atomic::Ordering;

    if config.api_key.is_empty() {
        return Err("API key is not configured".to_string());
    }

    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let body = serde_json::to_string(&ChatCompletionRequest {
        model: &config.model,
        messages,
        stream: true,
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
    })
    .map_err(|e| format!("Failed to serialize streaming request: {}", e))?;

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

    for line in reader.lines() {
        if let Some(flag) = cancel_flag {
            if flag.load(Ordering::Relaxed) {
                return Ok(full_text);
            }
        }

        let line = line.map_err(|e| format!("Stream read error: {}", e))?;
        let line = line.trim().to_string();

        if line.is_empty() {
            continue;
        }

        // SSE format: "data: {...}" or "data: [DONE]"
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                break;
            }
            if let Ok(chunk) = serde_json::from_str::<ChatStreamChunk>(data) {
                for choice in &chunk.choices {
                    if let Some(ref content) = choice.delta.content {
                        full_text.push_str(content);
                        on_token(content.clone());
                    }
                    if let Some(ref reason) = choice.finish_reason {
                        if reason == "stop" {
                            return Ok(full_text);
                        }
                    }
                }
            }
        }
    }

    Ok(full_text)
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
