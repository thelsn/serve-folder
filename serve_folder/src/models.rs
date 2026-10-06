use serde::{Serialize, Deserialize};

#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Serialize)]
pub struct DirResponse {
    pub current_path: String,
    pub entries: Vec<FileEntry>,
}

#[derive(Deserialize)]
pub struct StopRequest {
    pub confirm: bool,
}

#[derive(Serialize, Clone, Default)]
pub struct ZipProgress {
    pub current_file: String,
    pub processed_files: usize,
    pub total_files: usize,
    pub percentage: f32,
    /// The whole archive has been sent to the client
    pub done: bool,
    /// Zipping failed or the download was cancelled
    pub failed: bool,
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    // Optional so that requests without a path (e.g. the PWA share target) use the root
    #[serde(default)]
    pub path: String,
    pub operation_id: Option<String>,
}

#[derive(Deserialize)]
pub struct ProgressQuery {
    pub id: String,
}

// Error types
#[derive(Debug)]
pub struct CrossSiteRequest;
impl warp::reject::Reject for CrossSiteRequest {}
