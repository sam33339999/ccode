use std::sync::Arc;
use async_trait::async_trait;
use ccode_domain::cron::{CronJob, CronJobId};
use crate::PortError;

#[async_trait]
pub trait CronRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<CronJob>, PortError>;
    async fn find_by_id(&self, id: &CronJobId) -> Result<Option<CronJob>, PortError>;
    async fn save(&self, job: &CronJob) -> Result<(), PortError>;
    async fn delete(&self, id: &CronJobId) -> Result<(), PortError>;
}

#[async_trait]
impl<T: CronRepository + ?Sized> CronRepository for Arc<T> {
    async fn list(&self) -> Result<Vec<CronJob>, PortError> {
        (**self).list().await
    }
    async fn find_by_id(&self, id: &CronJobId) -> Result<Option<CronJob>, PortError> {
        (**self).find_by_id(id).await
    }
    async fn save(&self, job: &CronJob) -> Result<(), PortError> {
        (**self).save(job).await
    }
    async fn delete(&self, id: &CronJobId) -> Result<(), PortError> {
        (**self).delete(id).await
    }
}
