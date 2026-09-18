use crate::ai::api_client;

const EMBEDDING_DIM: usize = 384;

/// Lightweight fallback embedding engine.
///
/// SemanticDrive is API-first: production semantic vectors come from an
/// OpenAI-compatible embedding endpoint. This deterministic n-gram encoder
/// keeps filename/content search usable before an API key is configured and
/// avoids shipping or loading heavyweight local model runtimes.
pub struct EmbeddingEngine {
    dim: usize,
    instruction_prefix: String,
}

impl Default for EmbeddingEngine {
    fn default() -> Self { Self::new() }
}

impl EmbeddingEngine {
    pub fn new() -> Self {
        Self { dim: EMBEDDING_DIM, instruction_prefix: String::new() }
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self { instruction_prefix: prefix.to_string(), ..Self::new() }
    }

    /// Local model inference is intentionally not part of the API-first build.
    pub fn is_loaded(&self) -> bool { false }

    pub fn embed(&self, text: &str) -> Vec<f32> { self.fallback_embed(text) }

    pub fn embed_batch(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.iter().map(|text| self.embed(text)).collect()
    }

    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() { return 0.0; }
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
    }

    pub fn embed_batch_remote(&self, texts: &[String], config: &api_client::ApiEndpointConfig) -> Result<Vec<Vec<f32>>, String> {
        api_client::call_embedding_api(config, texts)
    }

    pub fn embed_remote(&self, text: &str, config: &api_client::ApiEndpointConfig) -> Result<Vec<f32>, String> {
        let mut vectors = self.embed_batch_remote(&[text.to_string()], config)?;
        vectors.pop().ok_or_else(|| "Embedding API returned no vector".to_string())
    }

    fn fallback_embed(&self, text: &str) -> Vec<f32> {
        let normalized = format!("{}{}", self.instruction_prefix, text.to_lowercase());
        let mut vector = vec![0.0f32; self.dim];
        let chars: Vec<char> = normalized.chars().collect();
        if chars.is_empty() { return vector; }
        for width in [1usize, 2, 3] {
            for window in chars.windows(width) {
                let mut hash = 5381u64;
                for ch in window {
                    for byte in ch.to_string().bytes() {
                        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
                    }
                }
                vector[(hash % self.dim as u64) as usize] += 1.0;
            }
        }
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut vector { *value /= norm; }
        }
        vector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_embeddings_are_normalized_and_stable() {
        let engine = EmbeddingEngine::new();
        let first = engine.embed("2026 年度财务报表");
        let second = engine.embed("2026 年度财务报表");
        let norm = first.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert_eq!(first, second);
    }
}
