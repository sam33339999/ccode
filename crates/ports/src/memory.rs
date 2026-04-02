use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::PortError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub session_id: Option<String>,
    pub created_at: u64,
    /// 任意附加資料（JSON object）
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait MemoryPort: Send + Sync {
    /// 儲存一筆記憶，回傳其 id（由實作自行生成 uuid 或自定格式）
    async fn store(&self, entry: MemoryEntry) -> Result<String, PortError>;
    /// 全文搜尋，回傳最相關的 limit 筆
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>, PortError>;
    /// 刪除指定 id
    async fn delete(&self, id: &str) -> Result<(), PortError>;
    /// 列出（可依 session_id 篩選），回傳最新的 limit 筆
    async fn list(&self, session_id: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>, PortError>;
}

// Arc blanket impl
#[async_trait]
impl<T: MemoryPort + ?Sized> MemoryPort for Arc<T> {
    async fn store(&self, entry: MemoryEntry) -> Result<String, PortError> { (**self).store(entry).await }
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>, PortError> { (**self).search(query, limit).await }
    async fn delete(&self, id: &str) -> Result<(), PortError> { (**self).delete(id).await }
    async fn list(&self, session_id: Option<&str>, limit: usize) -> Result<Vec<MemoryEntry>, PortError> { (**self).list(session_id, limit).await }
}
