use super::embedding::EmbeddingEngine;
use crate::scanner::FileEntry;
use std::collections::HashMap;

/// Represents a search result with relevance score
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub file_id: String,
    pub file_name: String,
    pub file_path: String,
    pub score: f32,
    pub match_type: String,
    pub snippet: String,
    pub file_size: u64,
    pub modified: String,
}

/// Hybrid search engine combining vector similarity and keyword matching.
///
/// Supports dual embedding models (zh + en) for cross-language search.
/// Both produce 768-dim vectors but in different spaces — they are compared
/// independently with the query embedded through the corresponding model.
pub struct SearchEngine {
    /// Chinese embedding engine (BGE-base-zh or fallback)
    zh_engine: EmbeddingEngine,
    /// English embedding engine (BGE-base-en, loaded on demand)
    en_engine: Option<EmbeddingEngine>,
    /// Cached embeddings from zh engine: file_id -> embedding vector
    embeddings_zh: HashMap<String, Vec<f32>>,
    /// Cached embeddings from en engine: file_id -> embedding vector
    embeddings_en: HashMap<String, Vec<f32>>,
    /// Cached content texts: file_id -> extracted text (shared)
    contents: HashMap<String, String>,
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            zh_engine: EmbeddingEngine::new(),
            en_engine: None,
            embeddings_zh: HashMap::new(),
            embeddings_en: HashMap::new(),
            contents: HashMap::new(),
        }
    }

    /// Index a file by storing its embedding vector (zh + en if available)
    pub fn index_file(&mut self, file_id: &str, text: &str) {
        let zh_emb = self.zh_engine.embed(text);
        self.embeddings_zh.insert(file_id.to_string(), zh_emb);
        self.contents.insert(file_id.to_string(), text.to_string());
        if let Some(ref en) = self.en_engine {
            let en_emb = en.embed(text);
            self.embeddings_en.insert(file_id.to_string(), en_emb);
        }
    }

    /// Batch-index multiple files with pre-computed embeddings.
    /// Minimizes lock hold time — caller computes embeddings outside the lock.
    /// Each entry: (file_id, text, zh_embedding, optional_en_embedding)
    pub fn index_files_batch(&mut self, entries: &[(String, String, Vec<f32>, Option<Vec<f32>>)]) {
        for (id, text, zh_emb, en_emb) in entries {
            self.embeddings_zh.insert(id.clone(), zh_emb.clone());
            self.contents.insert(id.clone(), text.clone());
            if let Some(emb) = en_emb {
                self.embeddings_en.insert(id.clone(), emb.clone());
            }
        }
    }

    /// Search files by natural language query.
    /// Returns results sorted by relevance score (highest first).
    /// Hybrid approach: combines vector similarity + filename/path keyword match + content keyword match.
    ///
    /// When `use_en` is true and the English embedding model is loaded, the query is
    /// embedded through both zh and en models independently. The max of the two vector
    /// similarities is used, enabling cross-language retrieval.
    pub fn search(
        &self,
        query: &str,
        files: &[FileEntry],
        max_results: usize,
        use_en: bool,
    ) -> Vec<SearchResult> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        let query_embedding_zh = self.zh_engine.embed(query);
        let query_embedding_en = if use_en {
            self.en_engine.as_ref().map(|e| e.embed(query))
        } else {
            None
        };

        let mut results: Vec<SearchResult> = files
            .iter()
            .filter_map(|file| {
                // 1a. Vector similarity via zh model (always available)
                let vector_score_zh = self.embeddings_zh
                    .get(&file.id)
                    .map(|doc_emb| EmbeddingEngine::cosine_similarity(&query_embedding_zh, doc_emb))
                    .unwrap_or(0.0);

                // 1b. Vector similarity via en model (if loaded and use_en is true)
                let vector_score_en = query_embedding_en.as_ref().and_then(|qe|
                    self.embeddings_en.get(&file.id)
                        .map(|doc_emb| EmbeddingEngine::cosine_similarity(qe, doc_emb))
                ).unwrap_or(0.0);

                // Take the max of both embedding spaces
                let vector_score = vector_score_zh.max(vector_score_en);

                // 2. Keyword match on filename and path
                let kw_score = keyword_score(query, &file.name, &file.path);

                // 3. Content keyword match (extracted text)
                let content_score = content_keyword_score(query, &self.contents, &file.id);

                // Combined score: adaptive weighted combination.
                // When vector is reliable (BGE model loaded), weight it heavily.
                // When vector is weak (n-gram fallback), keyword/content complement.
                let score = if vector_score > 0.3 {
                    (0.6 * vector_score + 0.2 * kw_score + 0.2 * content_score).min(1.0)
                } else {
                    (0.3 * vector_score + 0.35 * kw_score + 0.35 * content_score).min(1.0)
                };

                if score < 0.05 {
                    return None;
                }

                let (match_type, snippet) = if vector_score > 0.0 && vector_score >= kw_score && vector_score >= content_score {
                    ("语义匹配".to_string(), get_snippet(&self.contents, &file.id))
                } else if content_score > 0.0 {
                    ("内容匹配".to_string(), get_snippet(&self.contents, &file.id))
                } else if kw_score > 0.0 {
                    ("关键词匹配".to_string(), file.path.clone())
                } else {
                    ("混合匹配".to_string(), file.path.clone())
                };

                Some(SearchResult {
                    file_id: file.id.clone(),
                    file_name: file.name.clone(),
                    file_path: file.path.clone(),
                    score,
                    match_type,
                    snippet,
                    file_size: file.size,
                    modified: file.modified.clone(),
                })
            })
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(max_results);
        results
    }

    /// Get number of zh-indexed files
    pub fn indexed_count(&self) -> usize {
        self.embeddings_zh.len()
    }

    /// Get number of en-indexed files (0 if en engine not loaded)
    pub fn indexed_count_en(&self) -> usize {
        self.embeddings_en.len()
    }

    /// Access the zh embedding engine (always available)
    pub fn embedding_engine(&self) -> &EmbeddingEngine {
        &self.zh_engine
    }

    /// Access the zh embedding engine (mutable)
    pub fn embedding_mut(&mut self) -> &mut EmbeddingEngine {
        &mut self.zh_engine
    }

    /// Access the en embedding engine if loaded
    pub fn en_embedding_engine(&self) -> Option<&EmbeddingEngine> {
        self.en_engine.as_ref()
    }

    /// Access the en embedding engine if loaded (mutable)
    pub fn en_embedding_mut(&mut self) -> Option<&mut EmbeddingEngine> {
        self.en_engine.as_mut()
    }

    /// Load zh embedding model from directory (config.json, tokenizer.json, model weights)
    pub fn load_embedding_model(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.zh_engine.load(path)?;
        let old_embeddings: Vec<(String, String)> = self.contents.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.embeddings_zh.clear();
        for (id, text) in &old_embeddings {
            let emb = self.zh_engine.embed(text);
            self.embeddings_zh.insert(id.clone(), emb);
        }
        log::info!("Re-indexed {} files with new zh embedding model", old_embeddings.len());
        Ok(())
    }

    /// Load en embedding model (BGE-base-en) and re-index existing content.
    pub fn load_en_model(&mut self, path: &std::path::Path) -> Result<(), String> {
        let prefix = "Represent this sentence for searching relevant passages: ";
        let mut en = EmbeddingEngine::with_prefix(prefix);
        en.load(path)?;
        let old_contents: Vec<(String, String)> = self.contents.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.embeddings_en.clear();
        for (id, text) in &old_contents {
            let emb = en.embed(text);
            self.embeddings_en.insert(id.clone(), emb);
        }
        self.en_engine = Some(en);
        log::info!(
            "BGE-base-en loaded and re-indexed {} files",
            old_contents.len()
        );
        Ok(())
    }

    /// Clear all indexed data
    pub fn clear(&mut self) {
        self.embeddings_zh.clear();
        self.embeddings_en.clear();
        self.contents.clear();
    }
}

/// Split a query into search terms at Latin/CJK boundaries.
///
/// Enables Chinese keyword matching without a full segmentation library.
/// Handles mixed-language queries common in Chinese daily use:
///   - "python作业" → ["python", "作业"]
///   - "和xxx的合同" → ["xxx", "合同"]
///   - "季度总结" → ["季度总结"]
///   - "2025年合同" → ["2025", "年", "合同"]
pub fn split_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_is_cjk: Option<bool> = None;

    for c in query.chars() {
        let is_cjk = (c >= '\u{4e00}' && c <= '\u{9fff}')
                   || (c >= '\u{3000}' && c <= '\u{303f}');
        let is_latin = c.is_ascii_alphanumeric();
        let is_divider = matches!(c, '_' | '-' | '/' | '\\' | '.' | ',' | '、' | '，');

        // Dividers always split
        if is_divider {
            if !current.is_empty() {
                terms.push(current.clone());
                current.clear();
            }
            prev_is_cjk = None;
            continue;
        }

        // Spaces split
        if c == ' ' {
            if !current.is_empty() {
                terms.push(current.clone());
                current.clear();
            }
            prev_is_cjk = None;
            continue;
        }

        // Skip non-text chars (unless # or @ which are meaningful in queries)
        if !is_cjk && !is_latin && c != '#' && c != '@' {
            continue;
        }

        // Split at CJK ↔ Latin boundary
        let is_current_cjk = is_cjk;
        if let Some(prev_cjk) = prev_is_cjk {
            if prev_cjk != is_current_cjk && !current.is_empty() {
                terms.push(current.clone());
                current.clear();
            }
        }

        current.push(c);
        if is_cjk || is_latin {
            prev_is_cjk = Some(is_current_cjk);
        }
    }

    if !current.is_empty() {
        terms.push(current);
    }

    // Filter out single-character terms (likely noise)
    terms.retain(|t| t.chars().count() >= 2);
    terms
}

/// Simple keyword matching score — supports Chinese via boundary splitting.
fn keyword_score(query: &str, name: &str, path: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let name_lower = name.to_lowercase();
    let path_lower = path.to_lowercase();

    let mut score = 0.0f32;

    // 1. Full query match (handles English space-separated and exact filename searches)
    if name_lower == query_lower {
        score += 0.9;
    }
    if name_lower.contains(&query_lower) {
        score += 0.7;
    }
    if path_lower.contains(&query_lower) {
        score += 0.3;
    }

    // 2. English word-by-word matching (space-separated tokens)
    for q_word in query_lower.split_whitespace() {
        if q_word.len() < 2 { continue; }
        if name_lower.contains(q_word) {
            score += 0.2;
        }
        if path_lower.contains(q_word) {
            score += 0.1;
        }
    }

    // 3. Chinese term matching via Latin/CJK boundary splitting
    let terms = split_terms(&query_lower);
    if terms.len() > 1 || (terms.len() == 1 && terms[0] != query_lower) {
        for term in &terms {
            if term.len() < 2 { continue; }
            if name_lower.contains(term) {
                score += 0.3;
            }
            if path_lower.contains(term) {
                score += 0.15;
            }
        }
    }

    score.min(1.0)
}

/// Content keyword matching — supports Chinese via boundary splitting.
/// Each matched term contributes BM25-style TF score, combined with match ratio.
fn content_keyword_score(query: &str, contents: &HashMap<String, String>, file_id: &str) -> f32 {
    if let Some(content) = contents.get(file_id) {
        let content_lower = content.to_lowercase();
        let query_lower = query.to_lowercase();

        // 1. Full query substring match (handles English and exact Chinese matches)
        if content_lower.contains(&query_lower) {
            let tf = content_lower.matches(&query_lower).count() as f32;
            let k1 = 1.2;
            return (tf / (tf + k1)) * 0.75;
        }

        // 2. Term-by-term matching via boundary splitting
        let terms = split_terms(&query_lower);
        if terms.is_empty() {
            return 0.0;
        }

        let mut matched_terms = 0;
        let mut total_bm25 = 0.0;

        for term in &terms {
            if term.len() < 2 {
                continue;
            }
            if content_lower.contains(term) {
                matched_terms += 1;
                let tf = content_lower.matches(term).count() as f32;
                // BM25-inspired: tf / (tf + k1)
                let k1 = 1.2;
                total_bm25 += tf / (tf + k1);
            }
        }

        if matched_terms == 0 {
            return 0.0;
        }

        let match_ratio = matched_terms as f32 / terms.len() as f32;
        let avg_bm25 = total_bm25 / matched_terms as f32;
        match_ratio * avg_bm25 * 0.6
    } else {
        0.0
    }
}

fn get_snippet(contents: &HashMap<String, String>, file_id: &str) -> String {
    contents
        .get(file_id)
        .map(|text| {
            let chars: Vec<char> = text.chars().collect();
            let len = chars.len().min(200);
            chars[..len].iter().collect::<String>()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_score_exact_match() {
        let score = keyword_score("report.pdf", "report.pdf", "/docs/report.pdf");
        assert!(score >= 0.9, "exact filename match should score >= 0.9, got {}", score);
    }

    #[test]
    fn test_keyword_score_partial_match() {
        let score = keyword_score("report", "monthly_report.xlsx", "/docs/monthly_report.xlsx");
        assert!(score >= 0.7, "filename contains query should score >= 0.7, got {}", score);
    }

    #[test]
    fn test_keyword_score_path_match() {
        let score = keyword_score("budget", "spreadsheet.xlsx", "/finance/budget/2024.xlsx");
        assert!(score > 0.3, "path contains query should score > 0.3, got {}", score);
    }

    #[test]
    fn test_keyword_score_no_match() {
        let score = keyword_score("zzzzz", "hello.txt", "/tmp/hello.txt");
        assert_eq!(score, 0.0, "no match should score 0.0, got {}", score);
    }

    #[test]
    fn test_keyword_score_case_insensitive() {
        let score = keyword_score("REPORT", "Monthly_Report.pdf", "/docs/");
        assert!(score >= 0.7, "case-insensitive match should score >= 0.7, got {}", score);
    }

    #[test]
    fn test_keyword_score_multi_word() {
        let score = keyword_score("annual report", "annual_report_2024.pdf", "/docs/");
        assert!(score > 0.0, "multi-word query should match, got {}", score);
    }

    #[test]
    fn test_keyword_score_capped_at_one() {
        let score = keyword_score("file", "file", "file");
        // Exact match (0.9) + name contains (0.7) + path contains (0.3)
        assert!(score <= 1.0, "score should be capped at 1.0, got {}", score);
    }

    #[test]
    fn test_get_snippet_short_text() {
        let mut contents = HashMap::new();
        contents.insert("1".to_string(), "Hello World".to_string());
        assert_eq!(get_snippet(&contents, "1"), "Hello World");
    }

    #[test]
    fn test_get_snippet_long_text() {
        let mut contents = HashMap::new();
        let long = "a".repeat(500);
        contents.insert("1".to_string(), long.clone());
        let snippet = get_snippet(&contents, "1");
        assert_eq!(snippet.len(), 200);
        assert_eq!(&snippet[..], &long[..200]);
    }

    #[test]
    fn test_get_snippet_missing_id() {
        let contents = HashMap::new();
        assert_eq!(get_snippet(&contents, "nonexistent"), "");
    }

    #[test]
    fn test_search_empty_query() {
        let engine = SearchEngine::new();
        let results = engine.search("", &[], 10, false);
        assert!(results.is_empty(), "empty query should return no results");
    }

    #[test]
    fn test_search_whitespace_query() {
        let engine = SearchEngine::new();
        let results = engine.search("   ", &[], 10, false);
        assert!(results.is_empty(), "whitespace query should return no results");
    }

    #[test]
    fn test_search_no_files() {
        let engine = SearchEngine::new();
        let results = engine.search("test", &[], 10, false);
        assert!(results.is_empty(), "search with no files should return no results");
    }

    #[test]
    fn test_new_engine_indexed_count_zero() {
        let engine = SearchEngine::new();
        assert_eq!(engine.indexed_count(), 0);
    }

    #[test]
    fn test_index_file_increases_count() {
        let mut engine = SearchEngine::new();
        engine.index_file("1", "some content");
        assert_eq!(engine.indexed_count(), 1);
    }

    #[test]
    fn test_index_duplicate_id() {
        let mut engine = SearchEngine::new();
        engine.index_file("1", "hello");
        engine.index_file("1", "world");
        // Should overwrite, not increase count
        assert_eq!(engine.indexed_count(), 1);
    }

    #[test]
    fn test_clear() {
        let mut engine = SearchEngine::new();
        engine.index_file("1", "test");
        engine.clear();
        assert_eq!(engine.indexed_count(), 0);
    }

    // ── split_terms tests ──

    #[test]
    fn test_split_terms_mixed_cjk_latin() {
        let terms = split_terms("python作业");
        assert!(terms.contains(&"python".to_string()));
        assert!(terms.contains(&"作业".to_string()));
    }

    #[test]
    fn test_split_terms_chinese_only() {
        let terms = split_terms("季度总结");
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0], "季度总结");
    }

    #[test]
    fn test_split_terms_with_stops() {
        let terms = split_terms("xxx合同");
        assert!(terms.contains(&"xxx".to_string()) || terms.contains(&"合同".to_string()));
        assert!(terms.len() >= 2);
    }

    #[test]
    fn test_split_terms_year_month() {
        let terms = split_terms("2025年合同");
        // Latin/CJK boundary splits "2025" from the Chinese part
        assert!(terms.contains(&"2025".to_string()));
        // "年合同" stays together since we only split at CJK↔Latin boundaries
        assert!(terms.iter().any(|t| t.contains("年") && t.contains("合同")));
    }

    #[test]
    fn test_split_terms_english_only() {
        let terms = split_terms("python homework");
        assert!(terms.contains(&"python".to_string()));
        assert!(terms.contains(&"homework".to_string()));
    }

    #[test]
    fn test_keyword_score_chinese_mixed() {
        // "python" part should match the filename
        let score = keyword_score("python作业", "my_python_homework.py", "/docs/");
        assert!(score > 0.0, "chinese mixed query should score > 0.0, got {}", score);
    }

    #[test]
    fn test_keyword_score_chinese_exact() {
        let score = keyword_score("合同", "采购合同.pdf", "/docs/");
        assert!(score >= 0.7, "exact chinese match should score >= 0.7, got {}", score);
    }

    #[test]
    fn test_keyword_score_multi_term_path() {
        let score = keyword_score("python作业", "homework.txt", "/code/python/");
        assert!(score > 0.0, "path match should score > 0.0, got {}", score);
    }
}
