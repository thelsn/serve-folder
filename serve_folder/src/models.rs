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
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    pub path: String,
    pub operation_id: Option<String>,
}

#[derive(Deserialize)]
pub struct ProgressQuery {
    pub id: String,
}

// Error types
#[derive(Debug)]
pub struct ZipCreationError;
impl warp::reject::Reject for ZipCreationError {}
