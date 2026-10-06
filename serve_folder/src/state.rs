use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use warp::Filter;

use crate::models::ZipProgress;

// Progress entries that haven't been updated for this long are dropped, so abandoned
// operations (e.g. the client went away mid-download) don't accumulate forever
const PROGRESS_TTL: Duration = Duration::from_secs(60 * 60);

pub struct ServerStateInner {
    pub shutdown_tx: Option<oneshot::Sender<()>>,
    pub root_path: PathBuf,
    pub zip_progress: HashMap<String, (ZipProgress, Instant)>,
    pub next_operation: u64,
}

#[derive(Clone)]
pub struct ServerState {
    inner: Arc<Mutex<ServerStateInner>>,
}

impl ServerState {
    pub fn new(root_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ServerStateInner {
                shutdown_tx: None,
                root_path,
                zip_progress: HashMap::new(),
                next_operation: 0,
            })),
        }
    }

    pub fn set_shutdown_tx(&self, tx: oneshot::Sender<()>) {
        let mut state = self.inner.lock().unwrap();
        state.shutdown_tx = Some(tx);
    }

    pub fn new_operation_id(&self) -> String {
        let mut state = self.inner.lock().unwrap();
        state.next_operation += 1;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("zip_{}_{}", millis, state.next_operation)
    }

    pub fn update_progress(&self, operation_id: &str, progress: ZipProgress) {
        let mut state = self.inner.lock().unwrap();
        let now = Instant::now();
        state.zip_progress.retain(|_, (_, updated)| now.duration_since(*updated) < PROGRESS_TTL);
        state.zip_progress.insert(operation_id.to_string(), (progress, now));
    }

    pub fn get_progress(&self, operation_id: &str) -> Option<ZipProgress> {
        let state = self.inner.lock().unwrap();
        state.zip_progress.get(operation_id).map(|(progress, _)| progress.clone())
    }

    pub fn with_state(&self) -> impl Filter<Extract = (ServerState,), Error = std::convert::Infallible> + Clone {
        let state = self.clone();
        warp::any().map(move || state.clone())
    }

    pub fn get_root_path(&self) -> PathBuf {
        let state = self.inner.lock().unwrap();
        state.root_path.clone()
    }

    pub fn take_shutdown_tx(&self) -> Option<oneshot::Sender<()>> {
        let mut state = self.inner.lock().unwrap();
        state.shutdown_tx.take()
    }
}
