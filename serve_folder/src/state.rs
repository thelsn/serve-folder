use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::sync::oneshot;
use warp::Filter;

use crate::models::ZipProgress;

pub struct ServerStateInner {
    pub shutdown_tx: Option<oneshot::Sender<()>>,
    pub root_path: PathBuf,
    pub zip_progress: HashMap<String, ZipProgress>,
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
            })),
        }
    }

    pub fn set_shutdown_tx(&self, tx: oneshot::Sender<()>) {
        let mut state = self.inner.lock().unwrap();
        state.shutdown_tx = Some(tx);
    }

    pub fn update_progress(&self, operation_id: &str, progress: ZipProgress) {
        let mut state = self.inner.lock().unwrap();
        state.zip_progress.insert(operation_id.to_string(), progress);
    }

    pub fn get_progress(&self, operation_id: &str) -> Option<ZipProgress> {
        let state = self.inner.lock().unwrap();
        state.zip_progress.get(operation_id).cloned()
    }

    pub fn remove_progress(&self, operation_id: &str) {
        let mut state = self.inner.lock().unwrap();
        state.zip_progress.remove(operation_id);
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
