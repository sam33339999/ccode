use async_trait::async_trait;
use ccode_domain::session::{Session, SessionId, SessionSummary};
use ccode_ports::{PortError, repositories::SessionRepository};
use std::cmp::Reverse;
use std::path::PathBuf;

/// File-based session repository.
/// Each session is stored as `<dir>/<session_id>.json`.
/// Writes are atomic: written to `<id>.json.tmp` then renamed.
pub struct FileSessionRepo {
    dir: PathBuf,
}

impl FileSessionRepo {
    /// Create repository backed by `dir`, creating the directory if it doesn't exist.
    pub fn new(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: &SessionId) -> PathBuf {
        self.dir.join(format!("{}.json", id.0))
    }
}

#[async_trait]
impl SessionRepository for FileSessionRepo {
    async fn list(&self, limit: usize) -> Result<Vec<SessionSummary>, PortError> {
        let mut entries = tokio::fs::read_dir(&self.dir)
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;

        let mut summaries = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = tokio::fs::read(&path)
                .await
                .map_err(|e| PortError::Storage(e.to_string()))?;
            match serde_json::from_slice::<Session>(&data) {
                Ok(session) => summaries.push(SessionSummary::from(&session)),
                Err(e) => tracing::warn!("skipping corrupt session file {:?}: {e}", path),
            }
        }
        summaries.sort_by_key(|s| Reverse(s.updated_at));
        summaries.truncate(limit);
        Ok(summaries)
    }

    async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, PortError> {
        let path = self.path_for(id);
        match tokio::fs::read(&path).await {
            Ok(data) => {
                let session =
                    serde_json::from_slice(&data).map_err(|e| PortError::Storage(e.to_string()))?;
                Ok(Some(session))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PortError::Storage(e.to_string())),
        }
    }

    async fn save(&self, session: &Session) -> Result<(), PortError> {
        let path = self.path_for(&session.id);
        let tmp = path.with_extension("json.tmp");
        let data =
            serde_json::to_vec_pretty(session).map_err(|e| PortError::Storage(e.to_string()))?;
        tokio::fs::write(&tmp, &data)
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), PortError> {
        let path = self.path_for(id);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PortError::Storage(e.to_string())),
        }
    }
}
