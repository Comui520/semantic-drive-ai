use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use std::path::Path;
use crate::ai::search::split_terms;

/// Structured result from natural language query parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedQuery {
    /// Search keywords extracted from the query
    pub keywords: Vec<String>,
    /// Optional time range as (start, end) in YYYY-MM-DD format
    pub time_range: Option<(String, String)>,
    /// File type filters (e.g. "pdf", "docx", "xlsx")
    pub file_types: Vec<String>,
    /// Named entities (people, places, projects)
    pub entities: Vec<String>,
    /// Original query text
    pub original: String,
}

const MAX_GEN_TOKENS: usize = 128;
const MAX_CHAT_TOKENS: usize = 256;
const CHAT_PROMPT_MAX: usize = 2048;

/// Configuration for text generation (chat responses).
#[derive(Debug, Clone)]
pub struct GenerateConfig {
    pub max_tokens: usize,
    pub temperature: f64,
    pub repetition_penalty: f64,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            max_tokens: MAX_CHAT_TOKENS,
            temperature: 0.6,
            repetition_penalty: 1.2,
        }
    }
}

/// LLM engine for natural language query understanding.
///
/// Uses Qwen2.5 GGUF model via Candle quantized inference.
/// Falls back to rule-based parsing when the model is not loaded.
pub struct LlmEngine {
    model: Option<ModelWeights>,
    tokenizer: Option<tokenizers::Tokenizer>,
    device: Device,
    eos_token_id: u32,
}

impl LlmEngine {
    pub fn new() -> Self {
        Self {
            model: None,
            tokenizer: None,
            device: Device::Cpu,
            eos_token_id: 151645, // Qwen2.5 EOS token
        }
    }

    /// Returns true if the Qwen model is loaded.
    pub fn is_loaded(&self) -> bool {
        self.model.is_some() && self.tokenizer.is_some()
    }

    /// Load Qwen2.5 GGUF model from a directory containing:
    /// - `model.gguf` (Q4_K_M quantized weights)
    /// - `tokenizer.json` (HuggingFace tokenizer)
    #[allow(unused)]
    pub fn load(&mut self, model_dir: &Path) -> Result<(), String> {
        let gguf_path = model_dir.join("model.gguf");
        let tokenizer_path = model_dir.join("tokenizer.json");

        if !gguf_path.exists() {
            return Err(format!("GGUF model not found: {}", gguf_path.display()));
        }
        if !tokenizer_path.exists() {
            return Err(format!("Tokenizer not found: {}", tokenizer_path.display()));
        }

        // Load tokenizer
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        // Load GGUF model
        let mut file = std::fs::File::open(&gguf_path)
            .map_err(|e| format!("Cannot open model: {}", e))?;
        let content = candle_core::quantized::gguf_file::Content::read(&mut file)
            .map_err(|e| format!("Cannot read GGUF content: {}", e))?;

        // Read EOS token before consuming content
        let maybe_eos = content.metadata.get("tokenizer.ggml.eos_token_id");
        if let Some(v) = maybe_eos {
            if let Ok(id) = v.to_u32() {
                self.eos_token_id = id;
            }
        }

        let model = ModelWeights::from_gguf(content, &mut file, &self.device)
            .map_err(|e| format!("Cannot create model: {}", e))?;

        self.model = Some(model);
        self.tokenizer = Some(tokenizer);

        log::info!("Qwen2.5 LLM loaded successfully");
        Ok(())
    }

    /// Parse a natural language query into structured components.
    ///
    /// Uses the LLM when loaded, otherwise falls back to rule-based parsing.
    pub fn parse_query(&mut self, query: &str) -> ParsedQuery {
        if self.is_loaded() {
            self.parse_with_llm(query)
                .unwrap_or_else(|| self.fallback_parse(query))
        } else {
            self.fallback_parse(query)
        }
    }

    /// Generate text from a prompt. Returns the full generated text.
    /// This is a general-purpose generation method for chat responses.
    pub fn generate(&mut self, prompt: &str, config: &GenerateConfig) -> Result<String, String> {
        let model = self.model.as_mut().ok_or("Model not loaded")?;
        let tokenizer = self.tokenizer.as_ref().ok_or("Tokenizer not loaded")?;

        let encoding = tokenizer.encode(prompt, true).map_err(|e| format!("Encode error: {}", e))?;
        let mut tokens = encoding.get_ids().to_vec();

        // Truncate prompt if too long
        let prompt_max = CHAT_PROMPT_MAX.saturating_sub(config.max_tokens);
        if tokens.len() > prompt_max {
            tokens.truncate(prompt_max);
        }

        let prompt_len = tokens.len();

        // Pre-fill: process all prompt tokens at once
        let prompt_tensor = Tensor::new(&tokens[..], &self.device)
            .map_err(|e| format!("Tensor error: {}", e))?
            .unsqueeze(0).map_err(|e| format!("Unsqueeze error: {}", e))?;
        model.forward(&prompt_tensor, 0)
            .map_err(|e| format!("Forward error: {}", e))?;

        let vocab_size: usize = 151936; // Qwen2.5 vocab size, used for penalty bounds
        let mut past_tokens: Vec<u32> = Vec::new();

        // Auto-regressive generation
        for _ in 0..config.max_tokens {
            let last = *tokens.last().ok_or("Empty tokens")?;
            let input = Tensor::new(&[last], &self.device)
                .map_err(|e| format!("Tensor error: {}", e))?
                .unsqueeze(0).map_err(|e| format!("Unsqueeze error: {}", e))?;
            let logits = model.forward(&input, tokens.len() - 1)
                .map_err(|e| format!("Forward error: {}", e))?;

            let next = sample_next_token(&logits, config, &past_tokens, vocab_size)
                .ok_or("Sampling failed")?;

            if next == self.eos_token_id {
                break;
            }
            tokens.push(next);
            past_tokens.push(next);
        }

        // Decode only the generated portion
        let generated: Vec<u32> = tokens[prompt_len..].iter().copied().collect();
        tokenizer.decode(&generated, true).map_err(|e| format!("Decode error: {}", e))
    }

    /// Generate text with streaming via callback. Each decoded token chunk
    /// is passed to `on_token`. Returns the full generated text.
    /// If `cancel_flag` is provided and set to true, generation stops early.
    pub fn generate_streaming<F: FnMut(String)>(
        &mut self,
        prompt: &str,
        config: &GenerateConfig,
        cancel_flag: Option<&std::sync::atomic::AtomicBool>,
        on_token: &mut F,
    ) -> Result<String, String> {
        use std::sync::atomic::Ordering;
        let model = self.model.as_mut().ok_or("Model not loaded")?;
        let tokenizer = self.tokenizer.as_ref().ok_or("Tokenizer not loaded")?;

        let encoding = tokenizer.encode(prompt, true).map_err(|e| format!("Encode error: {}", e))?;
        let mut tokens = encoding.get_ids().to_vec();

        let prompt_max = CHAT_PROMPT_MAX.saturating_sub(config.max_tokens);
        if tokens.len() > prompt_max {
            tokens.truncate(prompt_max);
        }

        let prompt_len = tokens.len();

        // Pre-fill
        let prompt_tensor = Tensor::new(&tokens[..], &self.device)
            .map_err(|e| format!("Tensor error: {}", e))?
            .unsqueeze(0).map_err(|e| format!("Unsqueeze error: {}", e))?;
        model.forward(&prompt_tensor, 0)
            .map_err(|e| format!("Forward error: {}", e))?;

        // Buffer for batched token decode (avoid garbled multi-byte UTF-8)
        let mut token_buffer: Vec<u32> = Vec::with_capacity(8);
        let vocab_size: usize = 151936;
        let mut past_tokens: Vec<u32> = Vec::new();

        // Auto-regressive generation
        for _ in 0..config.max_tokens {
            // Check cancellation flag
            if let Some(flag) = cancel_flag {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
            }

            let last = *tokens.last().ok_or("Empty tokens")?;
            let input = Tensor::new(&[last], &self.device)
                .map_err(|e| format!("Tensor error: {}", e))?
                .unsqueeze(0).map_err(|e| format!("Unsqueeze error: {}", e))?;
            let logits = model.forward(&input, tokens.len() - 1)
                .map_err(|e| format!("Forward error: {}", e))?;

            let next = sample_next_token(&logits, config, &past_tokens, vocab_size)
                .ok_or("Sampling failed")?;

            if next == self.eos_token_id {
                break;
            }
            // Skip im_start special token to avoid garbled streaming output
            if next == 151644 {
                break;
            }
            tokens.push(next);
            past_tokens.push(next);
            token_buffer.push(next);

            // Batch-decode every 12 tokens for smoother UTF-8 output
            if token_buffer.len() >= 12 {
                if let Ok(text) = tokenizer.decode(&token_buffer, true) {
                    on_token(text);
                }
                token_buffer.clear();
            }
        }

        // Flush remaining buffered tokens
        if !token_buffer.is_empty() {
            if let Ok(text) = tokenizer.decode(&token_buffer, true) {
                on_token(text);
            }
        }

        // Decode full generated portion for return value
        let generated: Vec<u32> = tokens[prompt_len..].iter().copied().collect();
        tokenizer.decode(&generated, true).map_err(|e| format!("Decode error: {}", e))
    }

    fn parse_with_llm(&mut self, query: &str) -> Option<ParsedQuery> {
        let model = self.model.as_mut()?;
        let tokenizer = self.tokenizer.as_ref()?;

        let prompt = format!(
            "你是一个查询解析助手。从用户的中文搜索查询中提取结构化信息。\n\
            查询：{}\n\
            请以JSON格式输出：\n\
            {{ \"keywords\": [...], \"time_range\": [\"开始\", \"结束\"], \"file_types\": [...], \"entities\": [...] }}\n\
            只输出JSON：", query);

        let encoding = tokenizer.encode(prompt, true).ok()?;
        let mut tokens = encoding.get_ids().to_vec();
        if tokens.len() > 512 {
            tokens.truncate(512);
        }

        // Pre-fill: process all prompt tokens
        let prompt_tensor = Tensor::new(&tokens[..], &self.device).ok()?
            .unsqueeze(0).ok()?;
        let _ = model.forward(&prompt_tensor, 0).ok()?;

        // Auto-regressive generation
        for _ in 0..MAX_GEN_TOKENS {
            let last = *tokens.last()?;
            let input = Tensor::new(&[last], &self.device).ok()?
                .unsqueeze(0).ok()?;
            let logits = model.forward(&input, tokens.len() - 1).ok()?;

            // Greedy sampling
            let next = sample_argmax(&logits)?;
            if next == self.eos_token_id {
                break;
            }
            tokens.push(next);
        }

        // Extract only the generated portion (after prompt)
        let generated: Vec<u32> = tokens[encoding.get_ids().len().min(tokens.len())..]
            .iter().copied().collect();
        let output = tokenizer.decode(&generated, true).ok()?;

        // Parse JSON from output
        self.parse_json_response(&output)
    }

    /// Extract JSON object from LLM response text.
    fn parse_json_response(&self, text: &str) -> Option<ParsedQuery> {
        // Find JSON object boundaries
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        let json_str = &text[start..=end];

        #[derive(Deserialize)]
        struct RawQuery {
            #[serde(default)]
            keywords: Vec<String>,
            #[serde(default)]
            time_range: Option<Vec<String>>,
            #[serde(default)]
            file_types: Vec<String>,
            #[serde(default)]
            entities: Vec<String>,
        }

        // Try direct parse; if fails, attempt basic fixes
        let raw: RawQuery = serde_json::from_str(json_str)
            .or_else(|_| {
                // Try stripping trailing commas before ] or }
                let fixed = json_str
                    .replace(",\n]", "\n]")
                    .replace(", ]", "]")
                    .replace(",\n}", "\n}")
                    .replace(", }", "}");
                serde_json::from_str(&fixed)
            })
            .or_else(|_| {
                // Try replacing single quotes with double quotes
                let fixed = json_str
                    .replace("'", "\"")
                    .replace("\\\"", "'"); // unescape already-escaped
                serde_json::from_str(&fixed)
            })
            .ok()?;

        let time_range = raw.time_range.and_then(|v| {
            if v.len() >= 2 {
                Some((v[0].clone(), v[1].clone()))
            } else {
                None
            }
        });

        Some(ParsedQuery {
            keywords: raw.keywords,
            time_range,
            file_types: raw.file_types,
            entities: raw.entities,
            original: String::new(), // filled by caller
        })
    }

    // ── Rule-based fallback parsing ──

    fn fallback_parse(&self, query: &str) -> ParsedQuery {
        let query_trimmed = query.trim();

        // Extract file type hints
        let file_types = self.extract_file_types(query_trimmed);

        // Extract time range (removes matched phrases from query)
        let (time_range, remaining_after_time) = self.extract_time_range(query_trimmed);

        // Build keywords from remaining text
        let keywords = self.extract_keywords(&remaining_after_time, &file_types);

        // Simple entity extraction: 2+ char Chinese words
        let entities = self.extract_entities(query_trimmed);

        ParsedQuery {
            keywords,
            time_range,
            file_types,
            entities,
            original: query_trimmed.to_string(),
        }
    }

    fn extract_file_types(&self, query: &str) -> Vec<String> {
        // Chinese category → file extension mapping
        const CHINESE_CATEGORIES: &[(&[&str], &[&str])] = &[
            (&["图片", "照片", "图像", "截图", "壁纸", "摄影"], &["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico"]),
            (&["文档", "文字", "文稿", "文章"], &["pdf", "doc", "docx", "txt", "md", "rtf"]),
            (&["视频", "电影", "影片", "录制", "录像"], &["mp4", "avi", "mkv", "mov", "wmv", "flv", "webm"]),
            (&["音乐", "音频", "歌曲", "录音", "语音"], &["mp3", "wav", "flac", "aac", "ogg", "wma"]),
            (&["表格", "报表", "统计", "电子表格", "excel"], &["xls", "xlsx", "csv", "tsv"]),
            (&["代码", "程序", "脚本", "编程", "源码"], &["rs", "py", "js", "ts", "java", "go", "c", "cpp", "rb", "php", "swift"]),
            (&["演示", "幻灯片", "演示文稿", "演讲"], &["ppt", "pptx", "key"]),
            (&["压缩", "压缩包", "归档", "备份"], &["zip", "rar", "7z", "tar", "gz", "bz2"]),
            (&["电子书", "书籍", "小说", "图书", "读物"], &["epub", "mobi", "azw3", "pdf"]),
            (&["设计", "设计稿", "UI", "平面", "3D"], &["psd", "ai", "svg", "fig", "sketch"]),
        ];

        // Also handle explicit extension mentions (.pdf, .docx) and bare names (pdf, docx)
        let explicit_re = regex::Regex::new(r"\.(\w{2,4})").unwrap();
        let bare_extensions = ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
            "txt", "png", "jpg", "jpeg", "gif", "zip", "rar", "mp3", "mp4", "csv",
            "md", "html", "json", "xml", "py", "rs", "js", "ts", "go", "java"];
        let query_lower = query.to_lowercase();
        let mut types: Vec<String> = Vec::new();

        // Check Chinese categories first
        for (category_keywords, extensions) in CHINESE_CATEGORIES {
            if category_keywords.iter().any(|kw| query_lower.contains(kw)) {
                for ext in *extensions {
                    if !types.contains(&ext.to_string()) {
                        types.push(ext.to_string());
                    }
                }
            }
        }

        // Also extract explicit extensions like .pdf, .docx
        for cap in explicit_re.captures_iter(&query_lower) {
            let ext = cap[1].to_string();
            if !types.contains(&ext) {
                types.push(ext);
            }
        }

        // Detect bare extension names like "pdf", "docx" without dot prefix.
        // Use whole-word matching to avoid false positives like "python" matching "py".
        let terms = split_terms(&query_lower);
        for ext in &bare_extensions {
            if terms.iter().any(|t| t == ext) && !types.contains(&ext.to_string()) {
                types.push(ext.to_string());
            }
        }

        types
    }

    /// Extract time range from query. Returns (range, remaining_query).
    /// The remaining query has matched time phrases removed for keyword extraction.
    fn extract_time_range(&self, query: &str) -> (Option<(String, String)>, String) {
        use chrono::Local;
        let today = Local::now().naive_local().date();

        // Check each pattern in priority order (most specific first)
        // If a pattern matches, we remove that phrase from the query for keyword extraction

        // 1. Absolute date: YYYY年M月D日
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let d = caps[3].parse::<u32>().unwrap_or(1);
            if let Some(date) = chrono::NaiveDate::from_ymd_opt(y, m, d) {
                let ds = date.format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((ds.clone(), ds)), remaining);
            }
        }

        // 2. Year before: YYYY年以前 / YYYY年之前
        let re = regex::Regex::new(r"(\d{4})年(?:以前|之前)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let end = chrono::NaiveDate::from_ymd_opt(y - 1, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some(("".to_string(), end)), remaining);
        }

        // 3. Year after: YYYY年以后 / YYYY年之后
        let re = regex::Regex::new(r"(\d{4})年(?:以后|之后)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some((start, "".to_string())), remaining);
        }

        // 4. Year range: YYYY到YYYY
        let re = regex::Regex::new(r"(\d{4})到(\d{4})").unwrap();
        if let Some(caps) = re.captures(query) {
            let y1 = caps[1].parse::<i32>().unwrap_or(2026);
            let y2 = caps[2].parse::<i32>().unwrap_or(2026);
            let start = chrono::NaiveDate::from_ymd_opt(y1, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y2, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some((start, end)), remaining);
        }

        // 5. Year month with 以前/以后/之前/之后: must come before bare YYYY年M月
        // 5a. YYYY年M月以前/之前
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月(?:以前|之前)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let end = if m == 1 {
                chrono::NaiveDate::from_ymd_opt(y - 1, 12, 31).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap() - chrono::Duration::days(1)
            };
            let remaining = re.replace(query, "").to_string();
            return (Some(("".to_string(), end.format("%Y-%m-%d").to_string())), remaining);
        }

        // 5b. YYYY年M月以后/之后
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月(?:以后|之后)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let start = if m == 12 {
                chrono::NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap()
            };
            let remaining = re.replace(query, "").to_string();
            return (Some((start.format("%Y-%m-%d").to_string(), "".to_string())), remaining);
        }

        // 5c. M月以前/之前 (current year, only if we're past that month)
        let re = regex::Regex::new(r"(\d{1,2})月(?:以前|之前)").unwrap();
        if let Some(caps) = re.captures(query) {
            let m = caps[1].parse::<u32>().unwrap_or(1);
            let y = today.year();
            if m > 1 && today.month() > m {
                let end = chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap() - chrono::Duration::days(1);
                let remaining = re.replace(query, "").to_string();
                return (Some(("".to_string(), end.format("%Y-%m-%d").to_string())), remaining);
            } else if m == 1 && today.year() > y {
                let end = chrono::NaiveDate::from_ymd_opt(y - 1, 12, 31).unwrap();
                let remaining = re.replace(query, "").to_string();
                return (Some(("".to_string(), end.format("%Y-%m-%d").to_string())), remaining);
            }
            // Fall through if the time hasn't passed yet
        }

        // 5d. M月以后/之后 (current year)
        let re = regex::Regex::new(r"(\d{1,2})月(?:以后|之后)").unwrap();
        if let Some(caps) = re.captures(query) {
            let m = caps[1].parse::<u32>().unwrap_or(1);
            let y = today.year();
            if m < 12 {
                let start = chrono::NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap();
                let remaining = re.replace(query, "").to_string();
                return (Some((start.format("%Y-%m-%d").to_string(), "".to_string())), remaining);
            } else if m == 12 {
                let start = chrono::NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap();
                let remaining = re.replace(query, "").to_string();
                return (Some((start.format("%Y-%m-%d").to_string(), "".to_string())), remaining);
            }
        }

        // 5e. Bare 以前/之前 (everything before today)
        let re = regex::Regex::new(r"(?:以前|之前)").unwrap();
        if re.is_match(query) {
            let yesterday = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some(("".to_string(), yesterday)), remaining);
        }

        // 5. Year month: YYYY年M月 (must come before bare year)
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let start = chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap();
            let end = if m == 12 {
                chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap()
                    - chrono::Duration::days(1)
            };
            let remaining = re.replace(query, "").to_string();
            return (Some((
                start.format("%Y-%m-%d").to_string(),
                end.format("%Y-%m-%d").to_string(),
            )), remaining);
        }

        // 6. Bare year: YYYY年
        let re = regex::Regex::new(r"(\d{4})年").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some((start, end)), remaining);
        }

        // 7. 前年
        if query.contains("前年") {
            let y = today.year() - 2;
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = query.replace("前年", "");
            return (Some((start, end)), remaining);
        }

        // 8. 去年
        if query.contains("去年") {
            let y = today.year() - 1;
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = query.replace("去年", "");
            return (Some((start, end)), remaining);
        }

        // 9. 今年
        if query.contains("今年") {
            let y = today.year();
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = query.replace("今年", "");
            return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
        }

        // 10. 上上周
        if query.contains("上上周") || query.contains("上上週") {
            let start = (today - chrono::Duration::days(14)).format("%Y-%m-%d").to_string();
            let end = (today - chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
            let remaining = query.replace("上上周", "").replace("上上週", "");
            return (Some((start, end)), remaining);
        }

        // 11. 上周N / 上週N
        let re = regex::Regex::new(r"上[周週]([一二三四五六日])").unwrap();
        if let Some(caps) = re.captures(query) {
            let weekday_map = [("一", 1), ("二", 2), ("三", 3), ("四", 4), ("五", 5), ("六", 6), ("日", 7)];
            let target = &caps[1];
            for (name, num) in &weekday_map {
                if target.contains(name) {
                    let current_weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
                    let last_same_day = today - chrono::Duration::days(current_weekday + 7 - num);
                    let ds = last_same_day.format("%Y-%m-%d").to_string();
                    let remaining = re.replace(query, "").to_string();
                    return (Some((ds.clone(), ds)), remaining);
                }
            }
        }

        // 12. 本周 / 这周
        if query.contains("本周") || query.contains("这周") {
            let weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
            let monday = today - chrono::Duration::days(weekday - 1);
            let remaining = query.replace("本周", "").replace("这周", "");
            return (Some((
                monday.format("%Y-%m-%d").to_string(),
                today.format("%Y-%m-%d").to_string(),
            )), remaining);
        }

        // 13. 上周
        if query.contains("上周") || query.contains("上週") {
            let weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
            let last_monday = today - chrono::Duration::days(weekday + 6);
            let last_sunday = today - chrono::Duration::days(weekday);
            let remaining = query.replace("上周", "").replace("上週", "");
            return (Some((
                last_monday.format("%Y-%m-%d").to_string(),
                last_sunday.format("%Y-%m-%d").to_string(),
            )), remaining);
        }

        // 14. 前天
        if query.contains("前天") {
            let d = (today - chrono::Duration::days(2)).format("%Y-%m-%d").to_string();
            let remaining = query.replace("前天", "");
            return (Some((d.clone(), d)), remaining);
        }

        // 15. 今天
        if query.contains("今天") {
            let d = today.format("%Y-%m-%d").to_string();
            let remaining = query.replace("今天", "");
            return (Some((d.clone(), d)), remaining);
        }

        // 16. 昨天
        if query.contains("昨天") {
            let d = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
            let remaining = query.replace("昨天", "");
            return (Some((d.clone(), d)), remaining);
        }

        // 17. 本月 / 这个月
        if query.contains("本月") || query.contains("这个月") {
            let start = today.format("%Y-%m-01").to_string();
            let remaining = query.replace("本月", "").replace("这个月", "");
            return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
        }

        // 18. 上个月 / 上月
        if query.contains("上个月") || query.contains("上月") {
            let first_this = today.format("%Y-%m-01").to_string();
            if let Ok(first_this) = chrono::NaiveDate::parse_from_str(&first_this, "%Y-%m-%d") {
                let last_month_end = first_this - chrono::Duration::days(1);
                let last_month_start = last_month_end.format("%Y-%m-01").to_string();
                let remaining = query.replace("上个月", "").replace("上月", "");
                return (Some((
                    last_month_start,
                    last_month_end.format("%Y-%m-%d").to_string(),
                )), remaining);
            }
        }

        // 19. 最近N个月 / 近N个月
        let re = regex::Regex::new(r"(?:最近|近)(\d+)(?:个)?月").unwrap();
        if let Some(caps) = re.captures(query) {
            if let Ok(n) = caps[1].parse::<i64>() {
                let start = (today - chrono::Duration::days(n * 30)).format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
            }
        }

        // 20. 最近N周 / 近N周
        let re = regex::Regex::new(r"(?:最近|近)(\d+)周").unwrap();
        if let Some(caps) = re.captures(query) {
            if let Ok(n) = caps[1].parse::<i64>() {
                let start = (today - chrono::Duration::days(n * 7)).format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
            }
        }

        // 21. 最近N天 / 近N天
        let re = regex::Regex::new(r"(?:最近|近)(\d+)[天日]").unwrap();
        if let Some(caps) = re.captures(query) {
            if let Ok(n) = caps[1].parse::<i64>() {
                let start = (today - chrono::Duration::days(n)).format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
            }
        }

        (None, query.to_string())
    }

    fn extract_keywords(&self, query: &str, _file_types: &[String]) -> Vec<String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        // Remove time-related and file-type phrases from the text
        let removal_words = [
            // Time
            "今天", "昨天", "前天", "本周", "这周", "上周", "下周", "上上周",
            "本月", "这个月", "上月", "上个月", "下月", "今年", "去年", "前年",
            "最近", "近", "以前", "之前", "以后", "之后",
            // File type
            "文件", "文档", "表格", "图片", "照片", "视频", "音乐", "压缩",
            "目录", "内容", "资料", "信息", "数据", "文件夹",
            // Common query fillers
            "一份", "一个", "一篇", "一本", "一张", "一条",
            "未完成", "已完成", "进行中",
        ];
        let mut cleaned = trimmed.to_string();
        for w in &removal_words {
            cleaned = cleaned.replace(w, "");
        }

        // Remove stop characters (Chinese particles, punctuation, separators)
        // Only include characters that are always grammatical particles and never part of meaningful compounds.
        let stop_chars: Vec<char> = "的了和与或在是有没有很被把将从对到以为就都要让给向往用做查找搜看写改删 ，。？！、；：.!?,()[]{}'\"吗吧呢啊哦啦嘛这那".chars().collect();
        cleaned.retain(|c| !stop_chars.contains(&c));

        // Keep only alphanumeric, underscore, hyphen (drop any leftover special chars)
        let clean: String = cleaned.chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        cleaned = clean;

        let mut keywords: Vec<String> = Vec::new();

        // Extract 3+ digit numbers (years, dates — very important for search)
        let num_re = regex::Regex::new(r"\d{3,}").unwrap();
        for m in num_re.find_iter(&cleaned) {
            let s = m.as_str().to_string();
            if !keywords.contains(&s) {
                keywords.push(s);
            }
        }
        cleaned = num_re.replace_all(&cleaned, "").to_string();

        // Remaining text (2+ characters) as a keyword
        let cleaned = cleaned.trim();
        if cleaned.chars().count() >= 2 {
            keywords.push(cleaned.to_string());
        }

        // Also add split terms from the original query for better Chinese matching
        // Each term goes through the same cleaning pipeline as the main query
        let split_terms = split_terms(trimmed);
        for term in &split_terms {
            let mut ct = term.clone();
            for w in &removal_words {
                ct = ct.replace(w, "");
            }
            ct.retain(|c| !stop_chars.contains(&c));
            let ct: String = ct.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if ct.chars().count() >= 2 && !keywords.contains(&ct) {
                keywords.push(ct);
            }
        }

        keywords
    }

    fn extract_entities(&self, query: &str) -> Vec<String> {
        let mut entities = Vec::new();
        for c in query.chars() {
            if c.is_whitespace() {
                continue;
            }
        }

        // Find consecutive Chinese characters (2+ chars) as potential entities
        let mut current = String::new();
        for ch in query.chars() {
            if ch >= '\u{4e00}' && ch <= '\u{9fff}' {
                current.push(ch);
            } else {
                if current.len() >= 2 {
                    // Filter out common stop words
                    let stops = ["文件", "文档", "表格", "图片", "照片", "视频", "最近",
                        "今天", "昨天", "本周", "上周", "本月", "上月", "搜索", "查找", "找到"];
                    if !stops.contains(&current.as_str()) {
                        entities.push(current.clone());
                    }
                }
                current.clear();
            }
        }
        if current.len() >= 2 {
            entities.push(current);
        }

        entities
    }
}

/// Greedy argmax sampling from logits tensor of shape (1, 1, vocab_size).
fn sample_argmax(logits: &Tensor) -> Option<u32> {
    let logits = logits.squeeze(0).ok()?; // (1, vocab_size)
    let logits = logits.squeeze(0).ok()?; // (vocab_size,)
    let next = logits.argmax(0).ok()?;
    next.to_scalar::<u32>().ok()
}

/// Temperature-based multinomial sampling from logits tensor of shape (1, 1, vocab_size).
fn sample_with_temperature(logits: &Tensor, temperature: f64) -> Option<u32> {
    let logits = logits.squeeze(0).ok()?; // (1, vocab_size)
    let logits = logits.squeeze(0).ok()?; // (vocab_size,)
    let logits_vec: Vec<f32> = logits.to_vec1().ok()?;

    if logits_vec.is_empty() {
        return None;
    }

    let temp = temperature.max(0.01) as f32;

    // Softmax with temperature
    let max_val = logits_vec.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits_vec.iter()
        .map(|x| ((x - max_val) / temp).exp())
        .collect();
    let sum: f32 = exps.iter().sum();
    if sum <= 0.0 {
        // Fallback to argmax if all probs are zero
        return logits_vec.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i as u32);
    }

    // Multinomial sampling
    let r = rand::random::<f32>();
    let mut cumulative = 0.0;
    for (i, p) in exps.iter().enumerate() {
        cumulative += p / sum;
        if r < cumulative {
            return Some(i as u32);
        }
    }

    // Fallback to last token
    Some((logits_vec.len() - 1) as u32)
}

// ── Vec-based sampling with repetition penalty ──

/// Apply HuggingFace-style repetition penalty to logits.
/// For each token in `past_tokens`, its logit is divided by `penalty`.
/// penalty=1.0 means no penalty; penalty>1.0 suppresses repetition.
fn apply_repetition_penalty(
    logits: &mut [f32],
    past_tokens: &[u32],
    penalty: f64,
    vocab_size: usize,
) {
    let penalty_f32 = penalty as f32;
    if (penalty_f32 - 1.0).abs() < f32::EPSILON {
        return;
    }
    for &token in past_tokens {
        let idx = token as usize;
        if idx < vocab_size {
            let logit = logits[idx];
            logits[idx] = if logit > 0.0 {
                logit / penalty_f32
            } else {
                logit * penalty_f32
            };
        }
    }
}

/// Argmax from a pre-extracted logits vec.
fn sample_argmax_from_vec(logits: &[f32]) -> Option<u32> {
    logits.iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i as u32)
}

/// Temperature-based multinomial sampling from a pre-extracted logits vec.
fn sample_with_temperature_from_vec(logits: &[f32], temperature: f64) -> Option<u32> {
    if logits.is_empty() {
        return None;
    }
    let temp = temperature.max(0.01) as f32;
    let max_val = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter()
        .map(|x| ((x - max_val) / temp).exp())
        .collect();
    let sum: f32 = exps.iter().sum();
    if sum <= 0.0 {
        return sample_argmax_from_vec(logits);
    }
    let r = rand::random::<f32>();
    let mut cumulative = 0.0;
    for (i, p) in exps.iter().enumerate() {
        cumulative += p / sum;
        if r < cumulative {
            return Some(i as u32);
        }
    }
    Some((logits.len() - 1) as u32)
}

/// Extract logits tensor to vec, apply repetition penalty, and sample the next token.
fn sample_next_token(
    logits: &Tensor,
    config: &GenerateConfig,
    past_tokens: &[u32],
    vocab_size: usize,
) -> Option<u32> {
    let logits = logits.squeeze(0).ok()?;
    let logits = logits.squeeze(0).ok()?;
    let mut logits_vec: Vec<f32> = logits.to_vec1().ok()?;

    // Apply repetition penalty to break self-reinforcing loops
    if config.repetition_penalty > 1.0 {
        apply_repetition_penalty(&mut logits_vec, past_tokens, config.repetition_penalty, vocab_size);
    }

    if config.temperature <= 0.0 {
        sample_argmax_from_vec(&logits_vec)
    } else {
        sample_with_temperature_from_vec(&logits_vec, config.temperature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> LlmEngine {
        LlmEngine::new()
    }

    // ── extract_file_types tests ──

    #[test]
    fn test_extract_pdf() {
        let engine = make_engine();
        let types = engine.extract_file_types("给我找一份pdf文档");
        assert!(types.contains(&"pdf".to_string()), "should find pdf type");
    }

    #[test]
    fn test_extract_excel() {
        let engine = make_engine();
        let types = engine.extract_file_types("查找上个月的excel表格");
        assert!(types.contains(&"xlsx".to_string()), "should find xlsx type");
    }

    #[test]
    fn test_extract_image() {
        let engine = make_engine();
        let types = engine.extract_file_types("找一些照片和图片");
        assert!(types.contains(&"jpg".to_string()), "should find jpg type");
    }

    #[test]
    fn test_extract_no_type() {
        let engine = make_engine();
        let types = engine.extract_file_types("帮我搜索一下合同文件");
        assert!(types.is_empty(), "合同 should not match any type");
    }

    #[test]
    fn test_extract_multiple_types() {
        let engine = make_engine();
        let types = engine.extract_file_types("pdf和excel文档");
        assert!(types.contains(&"pdf".to_string()), "should find pdf");
        assert!(types.contains(&"xlsx".to_string()), "should find xlsx");
    }

    // ── extract_time_range tests ──

    #[test]
    fn test_today() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("今天修改的文件");
        assert!(range.is_some(), "should detect 今天");
        let (start, end) = range.unwrap();
        assert_eq!(start, end, "today range should be same day");
    }

    #[test]
    fn test_yesterday() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("昨天的文件");
        assert!(range.is_some(), "should detect 昨天");
    }

    #[test]
    fn test_this_week() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("本周的文档");
        assert!(range.is_some(), "should detect 本周");
        let (start, end) = range.unwrap();
        assert!(start <= end, "start should be <= end");
    }

    #[test]
    fn test_last_week() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("上周的表格");
        assert!(range.is_some(), "should detect 上周");
    }

    #[test]
    fn test_this_month() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("这个月的文件");
        assert!(range.is_some(), "should detect 这个月");
    }

    #[test]
    fn test_last_month() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("上个月的资料");
        assert!(range.is_some(), "should detect 上个月");
    }

    #[test]
    fn test_recent_days() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("最近3天的文件");
        assert!(range.is_some(), "should detect 最近N天");
        if let Some((start, end)) = range {
            assert!(start <= end, "start should be <= end");
        }
    }

    #[test]
    fn test_no_time_range() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("搜索所有合同");
        assert!(range.is_none(), "no time mention should return None");
    }

    // ── extract_keywords tests ──

    #[test]
    fn test_extract_keywords_simple() {
        let engine = make_engine();
        let kw = engine.extract_keywords("财务报表 2024", &[]);
        assert!(kw.contains(&"财务报表".to_string()), "should extract '财务报表'");
        assert!(kw.contains(&"2024".to_string()), "should extract '2024'");
    }

    #[test]
    fn test_extract_keywords_removes_stopwords() {
        let engine = make_engine();
        let kw = engine.extract_keywords("今天的文件", &[]);
        assert!(!kw.contains(&"今天".to_string()), "should remove '今天'");
        assert!(!kw.contains(&"文件".to_string()), "should remove '文件'");
    }

    #[test]
    fn test_extract_keywords_removes_type_words() {
        let engine = make_engine();
        let kw = engine.extract_keywords("我的照片", &[]);
        assert!(!kw.contains(&"照片".to_string()), "should remove '照片'");
    }

    #[test]
    fn test_extract_keywords_empty_after_time_removal() {
        // "的文件" is the remaining text after extract_time_range removes "2026年以前"
        // from "2026年以前的文件". It should yield empty keywords.
        let engine = make_engine();
        let kw = engine.extract_keywords("的文件", &[]);
        assert!(kw.is_empty(), "only stop words should give empty keywords, got {:?}", kw);
    }

    #[test]
    fn test_extract_keywords_meaningful_after_time_removal() {
        // After removing "去年", the remaining "的财务报表" should yield "财务报表"
        let engine = make_engine();
        let kw = engine.extract_keywords("的财务报表", &[]);
        assert!(kw.contains(&"财务报表".to_string()), "should extract '财务报表'");
    }

    #[test]
    fn test_extract_keywords_short_tokens() {
        let engine = make_engine();
        let kw = engine.extract_keywords("a b 文件", &[]);
        // "文件" removed as type word, "a" and "b" filtered as stop chars,
        // but "ab" concatenates into a 2-char token that passes
        assert!(kw.iter().all(|k| k.chars().count() >= 2), "no single-char keywords retained");
    }

    // ── extract_entities tests ──

    #[test]
    fn test_extract_entities_multi_word() {
        let engine = make_engine();
        let entities = engine.extract_entities("张三 项目报告");
        assert!(entities.contains(&"张三".to_string()), "should extract '张三'");
        assert!(entities.contains(&"项目报告".to_string()), "should extract '项目报告'");
    }

    #[test]
    fn test_extract_entities_skips_stops() {
        let engine = make_engine();
        let entities = engine.extract_entities("今天文件");
        assert!(!entities.contains(&"今天".to_string()), "should skip '今天'");
    }

    #[test]
    fn test_extract_entities_short_not_included() {
        let engine = make_engine();
        let entities = engine.extract_entities("a的b");
        assert!(!entities.contains(&"a".to_string()), "single char should be excluded");
    }

    // ── fallback_parse integration test ──

    #[test]
    fn test_fallback_parse_full_query() {
        let engine = make_engine();
        let result = engine.fallback_parse("上周 张三 修改的 pdf 文件");
        assert!(!result.keywords.is_empty(), "should extract keywords");
        assert!(result.file_types.contains(&"pdf".to_string()), "should detect pdf");
        assert!(result.time_range.is_some(), "should detect 上周");
        assert!(result.entities.contains(&"张三".to_string()), "should detect '张三'");
    }

    #[test]
    fn test_fallback_parse_simple() {
        let engine = make_engine();
        let result = engine.fallback_parse("人工智能");
        assert!(result.keywords.contains(&"人工智能".to_string()), "should extract keywords");
        assert!(result.time_range.is_none(), "no time mention");
        assert!(result.file_types.is_empty(), "no type mention");
    }

    // ── is_loaded tests ──

    #[test]
    fn test_not_loaded_by_default() {
        let engine = make_engine();
        assert!(!engine.is_loaded(), "engine should not be loaded by default");
    }

    // ── parse_query fallback test ──

    #[test]
    fn test_parse_query_uses_fallback_when_not_loaded() {
        let mut engine = make_engine();
        let result = engine.parse_query("搜索最近7天的合同文件");
        assert_eq!(result.original, "搜索最近7天的合同文件");
        // Should produce keywords
        assert!(!result.keywords.is_empty() || !result.file_types.is_empty());
    }

    // ── New date parsing tests ──

    #[test]
    fn test_year_month_before() {
        let engine = make_engine();
        let (range, remaining) = engine.extract_time_range("2026年4月以前的文件");
        assert!(range.is_some(), "should detect 2026年4月以前");
        if let Some((start, end)) = range {
            assert_eq!(start, "", "start should be empty (from beginning)");
            assert!(end.as_str() <= "2026-03-31", "end should be before April, got {}", end);
        }
        // "文件" should remain for keyword extraction
        assert!(!remaining.is_empty(), "should have remaining text");
    }

    #[test]
    fn test_year_month_after() {
        let engine = make_engine();
        let (range, remaining) = engine.extract_time_range("2026年4月以后的资料");
        assert!(range.is_some(), "should detect 2026年4月以后");
        if let Some((start, end)) = range {
            assert!(start.as_str() >= "2026-05-01", "start should be after April, got {}", start);
            assert_eq!(end, "", "end should be empty (unbounded)");
        }
        assert!(!remaining.is_empty(), "should have remaining text");
    }

    #[test]
    fn test_bare_month_before() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("4月以前的文件");
        // This might not match if current month <= 4
        // Just verify it doesn't crash and returns something sensible
        if let Some((start, end)) = range {
            assert_eq!(start, "", "before should have empty start");
            assert!(!end.is_empty(), "end should not be empty");
        }
    }

    #[test]
    fn test_bare_yiqian() {
        let engine = make_engine();
        let (range, _) = engine.extract_time_range("以前的照片");
        assert!(range.is_some(), "bare 以前 should detect");
        if let Some((start, end)) = range {
            assert_eq!(start, "", "start should be empty");
            assert!(!end.is_empty(), "end should be yesterday");
        }
    }

    #[test]
    fn test_extract_keywords_chinese_mixed() {
        let engine = make_engine();
        let kw = engine.extract_keywords("python作业", &[]);
        let all: String = kw.join(" ");
        assert!(all.contains("python") || all.contains("作业"),
            "chinese mixed query should produce split terms, got: {:?}", kw);
    }

    #[test]
    fn test_extract_keywords_contract_search() {
        let engine = make_engine();
        let kw = engine.extract_keywords("和xxx的一份合同", &[]);
        let all: String = kw.join(" ");
        assert!(all.contains("合同"), "should extract 合同, got: {:?}", kw);
    }

    #[test]
    fn test_extract_keywords_diary_search() {
        let engine = make_engine();
        let kw = engine.extract_keywords("张三写的日记", &[]);
        let all: String = kw.join(" ");
        assert!(all.contains("日记"), "should extract 日记, got: {:?}", kw);
    }

    #[test]
    fn test_extract_keywords_project_search() {
        let engine = make_engine();
        let kw = engine.extract_keywords("未完成的项目", &[]);
        let all: String = kw.join(" ");
        assert!(all.contains("项目"), "should extract 项目, got: {:?}", kw);
    }

    #[test]
    fn test_extract_file_types_images() {
        let engine = make_engine();
        let types = engine.extract_file_types("找一些图片");
        assert!(types.contains(&"jpg".to_string()), "should find image types");
        assert!(types.contains(&"png".to_string()), "should find png type");
    }

    #[test]
    fn test_extract_file_types_code() {
        let engine = make_engine();
        let types = engine.extract_file_types("python代码");
        assert!(types.contains(&"py".to_string()), "should find python code type");
    }

    #[test]
    fn test_extract_file_types_video() {
        let engine = make_engine();
        let types = engine.extract_file_types("看视频");
        assert!(types.contains(&"mp4".to_string()), "should find video types");
    }
}
