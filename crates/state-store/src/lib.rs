use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("storage error: {0}")]
    Storage(String),
}

#[async_trait]
pub trait TriggerTaskStateStore: Send + Sync {
    async fn load_trigger_tasks(&self) -> Result<Vec<Value>, StateStoreError>;
    async fn save_trigger_task(&self, id: &str, task: &Value) -> Result<(), StateStoreError>;
    async fn delete_trigger_task(&self, id: &str) -> Result<(), StateStoreError>;
}

pub struct FileTriggerTaskStateStore {
    dir: PathBuf,
}

impl FileTriggerTaskStateStore {
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = root.as_ref().join("trigger-tasks");
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

#[async_trait]
impl TriggerTaskStateStore for FileTriggerTaskStateStore {
    async fn load_trigger_tasks(&self) -> Result<Vec<Value>, StateStoreError> {
        let entries =
            std::fs::read_dir(&self.dir).map_err(|e| StateStoreError::Storage(e.to_string()))?;
        let mut tasks = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| StateStoreError::Storage(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let data = std::fs::read(&path).map_err(|e| StateStoreError::Storage(e.to_string()))?;
            match serde_json::from_slice::<Value>(&data) {
                Ok(task) => tasks.push(task),
                Err(err) => tracing::warn!("skipping corrupt trigger task file {:?}: {err}", path),
            }
        }

        tasks.sort_by(|a, b| {
            let lhs = a.get("id").and_then(Value::as_str).unwrap_or_default();
            let rhs = b.get("id").and_then(Value::as_str).unwrap_or_default();
            lhs.cmp(rhs)
        });
        Ok(tasks)
    }

    async fn save_trigger_task(&self, id: &str, task: &Value) -> Result<(), StateStoreError> {
        let path = self.path_for(id);
        let tmp = path.with_extension("json.tmp");
        let data =
            serde_json::to_vec_pretty(task).map_err(|e| StateStoreError::Storage(e.to_string()))?;
        std::fs::write(&tmp, data).map_err(|e| StateStoreError::Storage(e.to_string()))?;
        std::fs::rename(&tmp, &path).map_err(|e| StateStoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn delete_trigger_task(&self, id: &str) -> Result<(), StateStoreError> {
        let path = self.path_for(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StateStoreError::Storage(err.to_string())),
        }
    }
}
