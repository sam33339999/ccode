use crate::PortError;
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait EmbeddingPort: Send + Sync {
    fn name(&self) -> &str;
    /// 將單段文字嵌入為向量
    async fn embed(&self, text: &str) -> Result<Vec<f32>, PortError>;
    /// 批次嵌入
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PortError>;
}

#[async_trait]
impl<T: EmbeddingPort + ?Sized> EmbeddingPort for Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }
    async fn embed(&self, text: &str) -> Result<Vec<f32>, PortError> {
        (**self).embed(text).await
    }
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, PortError> {
        (**self).embed_batch(texts).await
    }
}
