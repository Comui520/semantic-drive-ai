use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use std::path::Path;

const EMBEDDING_DIM: usize = 384;
const MAX_SEQ_LEN: usize = 512;

/// Text embedding engine using BGE model via Candle (pure Rust).
///
/// Falls back to character n-gram hashing when the model is not loaded,
/// so the app remains functional without downloaded models.
pub struct EmbeddingEngine {
    model: Option<BertModel>,
    tokenizer: Option<tokenizers::Tokenizer>,
    device: Device,
    dim: usize,
    /// Instruction prefix prepended before each text for BGE models.
    /// Chinese: "为这个句子生成向量表示: "
    /// English: "Represent this sentence for searching relevant passages: "
    instruction_prefix: String,
}

impl EmbeddingEngine {
    pub fn new() -> Self {
        Self {
            model: None,
            tokenizer: None,
            device: Device::Cpu,
            dim: EMBEDDING_DIM,
            instruction_prefix: "为这个句子生成向量表示: ".to_string(),
        }
    }

    /// Create engine with a custom instruction prefix (e.g. for BGE-base-en).
    pub fn with_prefix(prefix: &str) -> Self {
        let mut s = Self::new();
        s.instruction_prefix = prefix.to_string();
        s
    }

    /// Returns true if the BGE model is loaded and ready for embedding.
    pub fn is_loaded(&self) -> bool {
        self.model.is_some() && self.tokenizer.is_some()
    }

    /// Load BGE model from a directory containing:
    /// - `tokenizer.json` (HuggingFace tokenizer)
    /// - `config.json` (BERT configuration)
    /// - `model.safetensors` (model weights)
    pub fn load(&mut self, model_dir: &Path) -> Result<(), String> {
        let tokenizer_path = model_dir.join("tokenizer.json");
        let config_path = model_dir.join("config.json");
        let safetensors_path = model_dir.join("model.safetensors");
        let pytorch_path = model_dir.join("pytorch_model.bin");

        if !tokenizer_path.exists() {
            return Err(format!("Tokenizer not found: {}", tokenizer_path.display()));
        }
        if !config_path.exists() {
            return Err(format!("Config not found: {}", config_path.display()));
        }
        if !safetensors_path.exists() && !pytorch_path.exists() {
            return Err(format!(
                "No model weights found at {} or {}",
                safetensors_path.display(),
                pytorch_path.display()
            ));
        }

        // Load tokenizer
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        // Load BERT config
        let config_content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config: {}", e))?;
        let config: BertConfig = serde_json::from_str(&config_content)
            .map_err(|e| format!("Failed to parse BERT config: {}", e))?;

        // Load model weights — try safetensors first (mmap, zero-copy), fall back to PyTorch
        let safetensors_valid = safetensors_path.exists()
            && safetensors_path.metadata().map(|m| m.len()).unwrap_or(0) > 1024;

        let vb = if safetensors_valid {
            let model_path_str = safetensors_path.to_string_lossy().to_string();
            unsafe {
                VarBuilder::from_mmaped_safetensors(
                    &[model_path_str],
                    candle_core::DType::F32,
                    &self.device,
                )
            }
            .map_err(|e| format!("Failed to load safetensors weights: {}", e))?
        } else if pytorch_path.exists() {
            VarBuilder::from_pth(
                &pytorch_path,
                candle_core::DType::F32,
                &self.device,
            )
            .map_err(|e| format!("Failed to load PyTorch weights: {}", e))?
        } else {
            return Err(format!(
                "No valid model weights found at {} (safetensors: {}b) or {}",
                safetensors_path.display(),
                safetensors_path.metadata().map(|m| m.len()).unwrap_or(0),
                pytorch_path.display(),
            ));
        };

        let model = BertModel::load(vb, &config)
            .map_err(|e| format!("Failed to create BERT model: {}", e))?;

        self.dim = config.hidden_size;
        self.model = Some(model);
        self.tokenizer = Some(tokenizer);

        log::info!(
            "BGE model loaded: dim={}, layers={}, heads={}",
            self.dim,
            config.num_hidden_layers,
            config.num_attention_heads
        );
        Ok(())
    }

    /// Embed text into a fixed-size vector.
    ///
    /// Uses BGE model when loaded; falls back to character n-gram hashing.
    pub fn embed(&self, text: &str) -> Vec<f32> {
        if let (Some(model), Some(tokenizer)) = (&self.model, &self.tokenizer) {
            self.embed_with_model(text, model, tokenizer)
                .unwrap_or_else(|| self.fallback_embed(text))
        } else {
            self.fallback_embed(text)
        }
    }

    /// Embed multiple texts in a single batched forward pass.
    /// Falls back to sequential embed() when model is not loaded.
    pub fn embed_batch(&self, texts: &[String]) -> Vec<Vec<f32>> {
        if texts.is_empty() {
            return Vec::new();
        }
        if let (Some(model), Some(tokenizer)) = (&self.model, &self.tokenizer) {
            self.embed_batch_with_model(texts, model, tokenizer)
                .unwrap_or_else(|| texts.iter().map(|t| self.fallback_embed(t)).collect())
        } else {
            texts.iter().map(|t| self.fallback_embed(t)).collect()
        }
    }

    /// Compute cosine similarity between two L2-normalized vectors.
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        dot.clamp(-1.0, 1.0)
    }

    // ── Private helpers ──

    /// BGE model-based embedding with mean pooling + L2 normalization.
    fn embed_with_model(
        &self,
        text: &str,
        model: &BertModel,
        tokenizer: &tokenizers::Tokenizer,
    ) -> Option<Vec<f32>> {
        // BGE instruction prefix (Chinese or English depending on model)
        let input = format!("{} {}", self.instruction_prefix, text);

        let encoding = tokenizer.encode(input, true).ok()?;
        let ids = encoding.get_ids();
        let mask = encoding.get_attention_mask();
        let type_ids = encoding.get_type_ids();

        // Truncate to max sequence length
        let len = ids.len().min(MAX_SEQ_LEN);
        let input_ids = Tensor::new(&ids[..len], &self.device).ok()?.unsqueeze(0).ok()?;
        let attention_mask = Tensor::new(&mask[..len], &self.device).ok()?.unsqueeze(0).ok()?;
        let token_type_ids = Tensor::new(&type_ids[..len], &self.device).ok()?.unsqueeze(0).ok()?;

        // Forward pass through BERT
        let last_hidden = model.forward(&input_ids, &attention_mask, Some(&token_type_ids)).ok()?;

        // Mean pooling + L2 normalization
        let pooled = mean_pooling(&last_hidden, &attention_mask).ok()?;
        let normalized = l2_normalize(&pooled).ok()?;

        // Convert to flat Vec<f32>
        normalized.flatten_all().ok()?.to_vec1().ok()
    }

    /// Batched BGE model inference — tokenizes all texts, pads to common length,
    /// runs a single forward pass, then splits pooled vectors per sequence.
    fn embed_batch_with_model(
        &self,
        texts: &[String],
        model: &BertModel,
        tokenizer: &tokenizers::Tokenizer,
    ) -> Option<Vec<Vec<f32>>> {
        // 1. Tokenize all texts
        let all_encodings: Vec<_> = texts.iter().map(|text| {
            let input = format!("{} {}", self.instruction_prefix, text);
            tokenizer.encode(input, true).ok()
        }).collect();

        // 2. Find max length for padding
        let max_len = all_encodings.iter()
            .filter_map(|e| e.as_ref().map(|e| e.get_ids().len().min(MAX_SEQ_LEN)))
            .max()?;

        let batch_size = texts.len();
        let mut input_ids_vec = vec![0u32; batch_size * max_len];
        let mut attention_mask_vec = vec![0u32; batch_size * max_len];
        let mut token_type_ids_vec = vec![0u32; batch_size * max_len];

        for (i, enc_opt) in all_encodings.iter().enumerate() {
            let enc = enc_opt.as_ref()?;
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let type_ids = enc.get_type_ids();
            let len = ids.len().min(max_len);
            let offset = i * max_len;
            input_ids_vec[offset..offset + len].copy_from_slice(&ids[..len]);
            attention_mask_vec[offset..offset + len].copy_from_slice(&mask[..len]);
            token_type_ids_vec[offset..offset + len].copy_from_slice(&type_ids[..len]);
        }

        // 3. Create batched tensors
        let input_ids = Tensor::from_slice(&input_ids_vec, (batch_size, max_len), &self.device).ok()?;
        let attention_mask = Tensor::from_slice(&attention_mask_vec, (batch_size, max_len), &self.device).ok()?;
        let token_type_ids = Tensor::from_slice(&token_type_ids_vec, (batch_size, max_len), &self.device).ok()?;

        // 4. Single batched forward pass
        let last_hidden = model.forward(&input_ids, &attention_mask, Some(&token_type_ids)).ok()?;

        // 5. Mean pooling + L2 normalize
        let pooled = mean_pooling(&last_hidden, &attention_mask).ok()?;
        let normalized = l2_normalize(&pooled).ok()?;

        // 6. Split into per-sequence vectors
        let flat: Vec<f32> = normalized.flatten_all().ok()?.to_vec1().ok()?;
        let dim = flat.len() / batch_size;
        let result: Vec<Vec<f32>> = (0..batch_size)
            .map(|i| flat[i * dim..(i + 1) * dim].to_vec())
            .collect();

        Some(result)
    }

    /// Fallback: character n-gram hashing (works without any model).
    fn fallback_embed(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dim];
        if text.is_empty() {
            return vec;
        }

        let chars: Vec<char> = text.chars().collect();
        for window_size in &[2, 3] {
            for window in chars.windows(*window_size) {
                let ngram: String = window.iter().collect();
                let hash = hash_str(&ngram);
                let idx = (hash % (self.dim as u64)) as usize;
                vec[idx] += 1.0;
            }
        }

        // L2 normalize
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        vec
    }
}

/// Mean pooling across the sequence dimension, weighted by attention mask.
fn mean_pooling(last_hidden: &Tensor, attention_mask: &Tensor) -> Result<Tensor, candle_core::Error> {
    let mask = attention_mask.unsqueeze(2)?.to_dtype(last_hidden.dtype())?;
    let masked = (last_hidden * mask.clone())?;
    let denom = mask.sum(1)?;
    masked.sum(1)?.broadcast_div(&denom)
}

/// L2 normalize along the last dimension.
fn l2_normalize(t: &Tensor) -> Result<Tensor, candle_core::Error> {
    let norm = t.sqr()?.sum_keepdim(1)?.sqrt()?;
    t.broadcast_div(&norm)
}

fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = EmbeddingEngine::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6, "identical vectors should have similarity 1.0");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = EmbeddingEngine::cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6, "orthogonal vectors should have similarity 0.0");
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = EmbeddingEngine::cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6, "opposite vectors should have similarity -1.0");
    }

    #[test]
    fn test_cosine_similarity_partial() {
        // cosine_similarity expects L2-normalized vectors — compute dot product only
        let a = vec![1.0, 0.0];
        let b = vec![0.6, 0.8]; // L2 normalized: sqrt(0.36+0.64)=1.0
        let sim = EmbeddingEngine::cosine_similarity(&a, &b);
        assert!((sim - 0.6).abs() < 1e-6, "expected 0.6, got {}", sim);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let sim = EmbeddingEngine::cosine_similarity(&[], &[]);
        assert_eq!(sim, 0.0, "empty vectors should return 0.0");
    }

    #[test]
    fn test_fallback_embed_non_empty() {
        let engine = EmbeddingEngine::new();
        let vec = engine.embed("测试文本");
        assert_eq!(vec.len(), EMBEDDING_DIM);
        assert!(vec.iter().any(|&v| v != 0.0), "embedding should not be all zeros");
    }

    #[test]
    fn test_fallback_embed_empty() {
        let engine = EmbeddingEngine::new();
        let vec = engine.embed("");
        assert_eq!(vec.len(), EMBEDDING_DIM);
        assert!(vec.iter().all(|&v| v == 0.0), "empty input should give all zeros");
    }

    #[test]
    fn test_fallback_embed_normalized() {
        let engine = EmbeddingEngine::new();
        let vec = engine.embed("normalize me");
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "output should be L2 normalized, norm={}", norm);
    }

    #[test]
    fn test_fallback_deterministic() {
        let engine = EmbeddingEngine::new();
        let a = engine.embed("相同的文本");
        let b = engine.embed("相同的文本");
        assert_eq!(a, b, "same input should produce same embedding");
    }

    #[test]
    fn test_fallback_different_inputs_different_vectors() {
        let engine = EmbeddingEngine::new();
        let a = engine.embed("财务报表");
        let b = engine.embed("小猫照片");
        let sim = EmbeddingEngine::cosine_similarity(&a, &b);
        // Even fallback should give different vectors for very different text
        assert!(sim < 0.9, "different inputs should have low similarity, got {}", sim);
    }

    #[test]
    fn test_hash_str_consistency() {
        let h1 = hash_str("hello");
        let h2 = hash_str("hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_str_different_inputs() {
        let h1 = hash_str("abc");
        let h2 = hash_str("xyz");
        assert_ne!(h1, h2, "different strings should produce different hashes");
    }

    #[test]
    fn test_hash_str_empty() {
        let h = hash_str("");
        assert_eq!(h, 5381, "empty string should return initial hash value");
    }
}
