use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use serde::{Deserialize, Serialize};
use std::path::Path;

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

    // ── LLM-based parsing ──

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

        let raw: RawQuery = serde_json::from_str(json_str).ok()?;
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

        // Extract time range
        let time_range = self.extract_time_range(query_trimmed);

        // Build keywords: remove known filter patterns
        let keywords = self.extract_keywords(query_trimmed, &file_types);

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
        let type_map = [
            ("pdf", ["pdf", "文档"].as_slice()),
            ("docx", ["word", "docx", "文档"].as_slice()),
            ("xlsx", ["excel", "xlsx", "xls", "表格", "电子表格"].as_slice()),
            ("pptx", ["ppt", "pptx", "演示", "幻灯片"].as_slice()),
            ("jpg", ["图片", "照片", "jpg", "png", "截图"].as_slice()),
            ("txt", ["txt", "文本", "记事本"].as_slice()),
            ("zip", ["zip", "压缩", "rar", "7z"].as_slice()),
            ("mp4", ["视频", "mp4", "avi", "mkv"].as_slice()),
            ("mp3", ["音乐", "mp3", "wav", "音频"].as_slice()),
        ];

        let query_lower = query.to_lowercase();
        let mut types = Vec::new();
        for (ext, keywords) in &type_map {
            for kw in *keywords {
                if query_lower.contains(kw) {
                    types.push(ext.to_string());
                    break;
                }
            }
        }
        types
    }

    fn extract_time_range(&self, query: &str) -> Option<(String, String)> {
        // Today's date for relative calculations
        use chrono::Local;
        let today = Local::now().naive_local().date();

        // "今天" → today
        if query.contains("今天") {
            let d = today.format("%Y-%m-%d").to_string();
            return Some((d.clone(), d));
        }
        // "昨天" → yesterday
        if query.contains("昨天") {
            let d = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
            return Some((d.clone(), d));
        }
        // "本周" / "这周" → this week (Monday to today)
        if query.contains("本周") || query.contains("这周") {
            let weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
            let monday = today - chrono::Duration::days(weekday - 1);
            return Some((
                monday.format("%Y-%m-%d").to_string(),
                today.format("%Y-%m-%d").to_string(),
            ));
        }
        // "上周" → last week
        if query.contains("上周") {
            let weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
            let last_monday = today - chrono::Duration::days(weekday + 6);
            let last_sunday = today - chrono::Duration::days(weekday);
            return Some((
                last_monday.format("%Y-%m-%d").to_string(),
                last_sunday.format("%Y-%m-%d").to_string(),
            ));
        }
        // "本月" → this month
        if query.contains("本月") || query.contains("这个月") {
            let start = today.format("%Y-%m-01").to_string();
            return Some((start, today.format("%Y-%m-%d").to_string()));
        }
        // "上月" → last month
        if query.contains("上月") || query.contains("上个月") {
            let first_this = today.format("%Y-%m-01").to_string();
            let first_this = chrono::NaiveDate::parse_from_str(&first_this, "%Y-%m-%d").ok()?;
            let last_month_end = first_this - chrono::Duration::days(1);
            let last_month_start = last_month_end.format("%Y-%m-01").to_string();
            return Some((
                last_month_start,
                last_month_end.format("%Y-%m-%d").to_string(),
            ));
        }
        // "最近N天" pattern
        if let Some(n) = self.parse_days(query, "最近") {
            let start = (today - chrono::Duration::days(n)).format("%Y-%m-%d").to_string();
            return Some((start, today.format("%Y-%m-%d").to_string()));
        }
        // "近N天" pattern
        if let Some(n) = self.parse_days(query, "近") {
            let start = (today - chrono::Duration::days(n)).format("%Y-%m-%d").to_string();
            return Some((start, today.format("%Y-%m-%d").to_string()));
        }

        None
    }

    fn parse_days(&self, query: &str, prefix: &str) -> Option<i64> {
        let query = query.replace(' ', "");
        if let Some(pos) = query.find(prefix) {
            let after = &query[pos + prefix.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !num_str.is_empty() {
                if after[num_str.len()..].starts_with('天') || after[num_str.len()..].starts_with('日') {
                    return num_str.parse::<i64>().ok();
                }
            }
        }
        None
    }

    fn extract_keywords(&self, query: &str, _file_types: &[String]) -> Vec<String> {
        let mut keywords: Vec<String> = Vec::new();

        // Remove known time-related phrases and file type words
        let stop_phrases = [
            "今天", "昨天", "本周", "这周", "上周", "本月", "这个月", "上月", "上个月",
            "最近", "近", "天", "日", "的",
        ];
        let type_words = ["文件", "文档", "表格", "图片", "照片", "视频", "音乐", "压缩"];

        for token in query.split_whitespace() {
            let cleaned: String = token.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if cleaned.is_empty() || cleaned.len() < 2 {
                continue;
            }
            if stop_phrases.contains(&cleaned.as_str()) {
                continue;
            }
            if type_words.contains(&cleaned.as_str()) {
                continue;
            }
            if !keywords.contains(&cleaned) {
                keywords.push(cleaned);
            }
        }

        if keywords.is_empty() {
            keywords.push(query.to_string());
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
        let range = engine.extract_time_range("今天修改的文件");
        assert!(range.is_some(), "should detect 今天");
        let (start, end) = range.unwrap();
        assert_eq!(start, end, "today range should be same day");
    }

    #[test]
    fn test_yesterday() {
        let engine = make_engine();
        let range = engine.extract_time_range("昨天的文件");
        assert!(range.is_some(), "should detect 昨天");
    }

    #[test]
    fn test_this_week() {
        let engine = make_engine();
        let range = engine.extract_time_range("本周的文档");
        assert!(range.is_some(), "should detect 本周");
        let (start, end) = range.unwrap();
        assert!(start <= end, "start should be <= end");
    }

    #[test]
    fn test_last_week() {
        let engine = make_engine();
        let range = engine.extract_time_range("上周的表格");
        assert!(range.is_some(), "should detect 上周");
    }

    #[test]
    fn test_this_month() {
        let engine = make_engine();
        let range = engine.extract_time_range("这个月的文件");
        assert!(range.is_some(), "should detect 这个月");
    }

    #[test]
    fn test_last_month() {
        let engine = make_engine();
        let range = engine.extract_time_range("上个月的资料");
        assert!(range.is_some(), "should detect 上个月");
    }

    #[test]
    fn test_recent_days() {
        let engine = make_engine();
        let range = engine.extract_time_range("最近3天的文件");
        assert!(range.is_some(), "should detect 最近N天");
        if let Some((start, end)) = range {
            assert!(start <= end, "start should be <= end");
        }
    }

    #[test]
    fn test_no_time_range() {
        let engine = make_engine();
        let range = engine.extract_time_range("搜索所有合同");
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
    fn test_extract_keywords_short_tokens() {
        let engine = make_engine();
        let kw = engine.extract_keywords("a b 文件", &[]);
        assert!(kw.is_empty() || kw.iter().all(|k| k.len() >= 2), "should filter short tokens");
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

    // ── parse_days tests ──

    #[test]
    fn test_parse_days_最近() {
        let engine = make_engine();
        let n = engine.parse_days("最近7天", "最近");
        assert_eq!(n, Some(7));
    }

    #[test]
    fn test_parse_days_近() {
        let engine = make_engine();
        let n = engine.parse_days("近30天", "近");
        assert_eq!(n, Some(30));
    }

    #[test]
    fn test_parse_days_no_match() {
        let engine = make_engine();
        let n = engine.parse_days("没有天数", "最近");
        assert_eq!(n, None);
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
        let result = engine.fallback_parse("财务报表");
        assert!(result.keywords.contains(&"财务报表".to_string()), "should extract keywords");
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
}
