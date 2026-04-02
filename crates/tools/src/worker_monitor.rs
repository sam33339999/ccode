use std::sync::{Arc, OnceLock};
use std::time::SystemTime;
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct WorkerMonitorEvent {
    pub task_id: String,
    pub status: String,
    pub summary: Option<String>,
    pub timestamp: SystemTime,
}

fn monitor_sender() -> &'static broadcast::Sender<Arc<WorkerMonitorEvent>> {
    static SENDER: OnceLock<broadcast::Sender<Arc<WorkerMonitorEvent>>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, _) = broadcast::channel(1024);
        tx
    })
}

pub fn publish_worker_event(event: WorkerMonitorEvent) {
    let _ = monitor_sender().send(Arc::new(event));
}

pub fn subscribe_worker_events() -> broadcast::Receiver<Arc<WorkerMonitorEvent>> {
    monitor_sender().subscribe()
}
