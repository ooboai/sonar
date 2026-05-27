use std::path::Path;

use anyhow::{Context, Result};
use model2vec_rs::model::StaticModel;

const DEFAULT_MODEL: &str = "minishlab/potion-code-16M";

pub struct Embedder {
    model: StaticModel,
    dim: usize,
}

impl Embedder {
    pub fn from_pretrained(model_id: &str) -> Result<Self> {
        let model = StaticModel::from_pretrained(model_id, None, Some(true), None)
            .context("failed to load model from HuggingFace Hub")?;
        let dim = model.encode(&["test".to_string()])[0].len();
        Ok(Self { model, dim })
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let model = StaticModel::from_pretrained(
            path.to_str().unwrap_or("."), None, Some(true), None,
        ).context("failed to load model from local path")?;
        let dim = model.encode(&["test".to_string()])[0].len();
        Ok(Self { model, dim })
    }

    pub fn load_default() -> Result<Self> {
        Self::from_pretrained(DEFAULT_MODEL)
    }

    pub fn dim(&self) -> usize { self.dim }

    pub fn encode(&self, text: &str) -> Vec<f32> {
        self.model.encode(&[text.to_string()]).into_iter().next().unwrap_or_default()
    }

    pub fn encode_batch(&self, texts: &[String]) -> Vec<Vec<f32>> {
        self.model.encode(texts)
    }
}

impl std::fmt::Debug for Embedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Embedder").field("dim", &self.dim).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_does_not_panic() {
        drop(Embedder::load_default());
    }
}
