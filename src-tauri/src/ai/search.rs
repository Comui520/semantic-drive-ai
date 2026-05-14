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

/// Hybrid search engine combining vector similarity and keyword matching
pub struct SearchEngine {
    engine: EmbeddingEngine,
    /// Cached embeddings: file_id -> embedding vector
    embeddings: HashMap<String, Vec<f32>>,
    /// Cached content texts: file_id -> extracted text
    contents: HashMap<String, String>,
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            engine: EmbeddingEngine::new(),
            embeddings: HashMap::new(),
            contents: HashMap::new(),
        }
    }

    /// Index a file by storing its embedding vector
    pub fn index_file(&mut self, file_id: &str, text: &str) {
        let embedding = self.engine.embed(text);
        self.embeddings.insert(file_id.to_string(), embedding);
        self.contents.insert(file_id.to_string(), text.to_string());
    }

    /// Search files by natural language query.
    /// Returns results sorted by relevance score (highest first).
    pub fn search(
        &self,
        query: &str,
        files: &[FileEntry],
        max_results: usize,
    ) -> Vec<SearchResult> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        let query_embedding = self.engine.embed(query);

        let mut results: Vec<SearchResult> = files
            .iter()
            .filter_map(|file| {
                let score = if let Some(doc_emb) = self.embeddings.get(&file.id) {
                    // Vector similarity
                    EmbeddingEngine::cosine_similarity(&query_embedding, doc_emb)
                } else {
                    // Fallback to keyword matching
                    keyword_score(query, &file.name, &file.path)
                };

                if score < 0.05 {
                    return None;
                }

                let (match_type, snippet) = if self.embeddings.contains_key(&file.id) {
                    ("语义匹配".to_string(), get_snippet(&self.contents, &file.id))
                } else {
                    ("关键词匹配".to_string(), file.path.clone())
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

    /// Get number of indexed files
    pub fn indexed_count(&self) -> usize {
        self.embeddings.len()
    }

    /// Access the underlying embedding engine
    pub fn embedding_engine(&self) -> &EmbeddingEngine {
        &self.engine
    }

    /// Access the underlying embedding engine (mutable)
    pub fn embedding_mut(&mut self) -> &mut EmbeddingEngine {
        &mut self.engine
    }

    /// Load embedding model from directory (containing config.json, tokenizer.json, model.safetensors)
    pub fn load_embedding_model(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.engine.load(path)?;
        // Re-index all cached content with the new model
        let old_embeddings: Vec<(String, String)> = self.contents.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.embeddings.clear();
        for (id, text) in &old_embeddings {
            let emb = self.engine.embed(text);
            self.embeddings.insert(id.clone(), emb);
        }
        log::info!("Re-indexed {} files with new embedding model", old_embeddings.len());
        Ok(())
    }

    /// Clear all indexed data
    pub fn clear(&mut self) {
        self.embeddings.clear();
        self.contents.clear();
    }
}

/// Simple keyword matching score
fn keyword_score(query: &str, name: &str, path: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let name_lower = name.to_lowercase();
    let path_lower = path.to_lowercase();

    let mut score = 0.0f32;

    // Exact filename match gives high score
    if name_lower == query_lower {
        score += 0.9;
    }

    // Filename contains query
    if name_lower.contains(&query_lower) {
        score += 0.7;
    }

    // Path contains query
    if path_lower.contains(&query_lower) {
        score += 0.3;
    }

    // Individual word matching
    for q_word in query_lower.split_whitespace() {
        if name_lower.contains(q_word) {
            score += 0.2;
        }
        if path_lower.contains(q_word) {
            score += 0.1;
        }
    }

    score.min(1.0)
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
        let results = engine.search("", &[], 10);
        assert!(results.is_empty(), "empty query should return no results");
    }

    #[test]
    fn test_search_whitespace_query() {
        let engine = SearchEngine::new();
        let results = engine.search("   ", &[], 10);
        assert!(results.is_empty(), "whitespace query should return no results");
    }

    #[test]
    fn test_search_no_files() {
        let engine = SearchEngine::new();
        let results = engine.search("test", &[], 10);
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
}
