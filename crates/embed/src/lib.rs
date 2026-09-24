//! candle-based embedding model (SPEC section 6). Weights and tokenizer are
//! embedded in the binary at compile time; `crates/embed/build.rs` fails
//! with a clear message if `cargo xtask fetch-model` hasn't been run.

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use sha2::{Digest, Sha256};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("model error: {0}")]
    Candle(#[from] candle_core::Error),
    #[error("tokenizer error: {0}")]
    Tokenizer(String),
    #[error("model config error: {0}")]
    Config(#[from] serde_json::Error),
}

/// Swappable embedding model (SPEC section 6): when the active model
/// changes, callers re-embed all documents as a background job and keep
/// the old vectors until it completes.
pub trait Embedder {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

const MODEL_SAFETENSORS: &[u8] =
    include_bytes!(concat!(env!("MODEL_DIR"), "/model_f16.safetensors"));
const TOKENIZER_JSON: &[u8] = include_bytes!(concat!(env!("MODEL_DIR"), "/tokenizer.json"));
const CONFIG_JSON: &str = include_str!(concat!(env!("MODEL_DIR"), "/config.json"));

pub struct BgeSmallEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    dim: usize,
    model_id: String,
}

impl BgeSmallEmbedder {
    /// Loads the embedded weights and tokenizer. Computation happens in
    /// f32 even though the stored weights are f16 (SPEC section 6);
    /// candle converts on load.
    pub fn load() -> Result<Self, EmbedError> {
        let config: BertConfig = serde_json::from_str(CONFIG_JSON)?;
        let device = Device::Cpu;

        let vb =
            VarBuilder::from_buffered_safetensors(MODEL_SAFETENSORS.to_vec(), DType::F32, &device)?;
        let model = BertModel::load(vb, &config)?;

        let mut tokenizer = Tokenizer::from_bytes(TOKENIZER_JSON)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 512,
                ..Default::default()
            }))
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        tokenizer.with_padding(Some(PaddingParams::default()));

        let model_id = format!(
            "bge-small-en-v1.5-f16@{}",
            &hex_sha256(MODEL_SAFETENSORS)[..12]
        );

        Ok(Self {
            model,
            tokenizer,
            device,
            dim: config.hidden_size,
            model_id,
        })
    }
}

impl Embedder for BgeSmallEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;

        let seq_len = encodings[0].get_ids().len();
        let mut input_ids = Vec::with_capacity(texts.len() * seq_len);
        let mut token_type_ids = Vec::with_capacity(texts.len() * seq_len);
        let mut attention_mask = Vec::with_capacity(texts.len() * seq_len);
        for encoding in &encodings {
            input_ids.extend(encoding.get_ids().iter().copied());
            token_type_ids.extend(encoding.get_type_ids().iter().copied());
            attention_mask.extend(encoding.get_attention_mask().iter().copied());
        }

        let shape = (texts.len(), seq_len);
        let input_ids = Tensor::from_vec(input_ids, shape, &self.device)?;
        let token_type_ids = Tensor::from_vec(token_type_ids, shape, &self.device)?;
        let attention_mask = Tensor::from_vec(attention_mask, shape, &self.device)?;

        let sequence_output =
            self.model
                .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

        // CLS pooling (SPEC section 6): the first token of each sequence.
        let cls = sequence_output.narrow(1, 0, 1)?.squeeze(1)?;

        // L2 normalisation.
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalised = cls.broadcast_div(&norm)?;

        Ok(normalised.to_vec2::<f32>()?)
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_embeds_with_the_right_shape() {
        let embedder = BgeSmallEmbedder::load().expect("model should load");
        assert_eq!(embedder.dim(), 384);
        assert!(embedder.model_id().starts_with("bge-small-en-v1.5-f16@"));

        let vectors = embedder
            .embed(&["A gadget. This is an abstract about gadgets.".to_string()])
            .expect("embedding should succeed");
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].len(), 384);

        let norm: f32 = vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "expected L2-normalised output, got norm {norm}"
        );
    }

    #[test]
    fn batching_multiple_texts_matches_embedding_them_one_at_a_time() {
        let embedder = BgeSmallEmbedder::load().expect("model should load");
        let texts = vec![
            "A gadget. An abstract about gadgets.".to_string(),
            "A much longer document. It describes, in considerable detail, a complex \
             apparatus for manufacturing widgets from raw materials, with many claims."
                .to_string(),
        ];

        let batched = embedder
            .embed(&texts)
            .expect("batch embedding should succeed");
        let individually: Vec<Vec<f32>> = texts
            .iter()
            .map(|t| {
                embedder
                    .embed(std::slice::from_ref(t))
                    .expect("single embedding should succeed")[0]
                    .clone()
            })
            .collect();

        for (b, i) in batched.iter().zip(individually.iter()) {
            let cosine: f32 = b.iter().zip(i.iter()).map(|(x, y)| x * y).sum();
            assert!(
                cosine > 0.999,
                "batched vs individual cosine similarity was {cosine}"
            );
        }
    }

    #[test]
    fn empty_input_returns_no_vectors() {
        let embedder = BgeSmallEmbedder::load().expect("model should load");
        assert_eq!(
            embedder.embed(&[]).expect("should succeed"),
            Vec::<Vec<f32>>::new()
        );
    }
}
