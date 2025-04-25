use std::path::Path;
use std::fs;
use std::io::Read;
use warp::{Reply, Rejection, http::HeaderValue};
use tempfile::NamedTempFile;

use crate::models::{FileEntry, DirResponse, StopRequest, DownloadQuery, ProgressQuery, ZipCreationError};
use crate::state::ServerState;
use crate::zip::{count_files_in_directory, create_zip_archive};

pub async fn handle_list(query: DownloadQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    // Get root path
    let root_path = state.get_root_path();
    
    // Process path
    let relative_path = query.path;
    let target_path = if relative_path.is_empty() {
        root_path.clone()
    } else {
        // Sanitize and validate the path
        let path = Path::new(&relative_path);
        let mut full_path = root_path.clone();
        for component in path.components() {
            match component {
                std::path::Component::Normal(name) => full_path.push(name),
                _ => continue, // Skip other components for security
            }
        }
        
        // Safety check
        if !full_path.starts_with(&root_path) {
            full_path = root_path.clone();
        }
        full_path
    };
    
    // Read directory contents
    let entries = match fs::read_dir(&target_path) {
        Ok(read_dir) => {
            let mut entries = Vec::new();
            for entry in read_dir {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    let metadata = match fs::metadata(&path) {
                        Ok(meta) => meta,
                        Err(_) => continue,
                    };
                    
                    // Get relative path from root
                    let rel_path = path.strip_prefix(&root_path).unwrap_or(&path);
                    let path_str = rel_path.to_string_lossy().to_string();
                    
                    entries.push(FileEntry {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: path_str,
                        is_dir: metadata.is_dir(),
                        size: if metadata.is_file() { metadata.len() } else { 0 },
                    });
                }
            }
            
            // Sort entries: directories first, then files
            entries.sort_by(|a, b| {
                if a.is_dir && !b.is_dir {
                    std::cmp::Ordering::Less
                } else if !a.is_dir && b.is_dir {
                    std::cmp::Ordering::Greater
                } else {
                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                }
            });
            
            entries
        },
        Err(_) => Vec::new(),
    };
    
    let rel_current = target_path.strip_prefix(&root_path).unwrap_or(Path::new(""));
    let current_path = rel_current.to_string_lossy().to_string();
    
    let response = DirResponse {
        current_path,
        entries,
    };
    
    Ok(warp::reply::json(&response))
}

pub async fn handle_stop(_stop_req: StopRequest, state: ServerState) -> Result<impl Reply, Rejection> {
    let tx = state.take_shutdown_tx();
    
    if let Some(tx) = tx {
        // Spawn a new task to send the stop signal after we've responded
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let _ = tx.send(());
        });
        
        return Ok(warp::reply::json(&serde_json::json!({
            "success": true,
            "message": "Server is shutting down"
        })));
    }
    
    Ok(warp::reply::json(&serde_json::json!({
        "success": false,
        "message": "Failed to stop server"
    })))
}

pub async fn handle_zip_progress(query: ProgressQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    let progress = state.get_progress(&query.id).unwrap_or_default();
    Ok(warp::reply::json(&progress))
}

pub async fn handle_zip_init(query: DownloadQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    let root_path = state.get_root_path();
    
    // Validate path
    let path = Path::new(&query.path);
    let mut full_path = root_path.clone();
    for component in path.components() {
        match component {
            std::path::Component::Normal(name) => full_path.push(name),
            _ => continue,
        }
    }
    
    if !full_path.starts_with(&root_path) || !full_path.is_dir() {
        return Err(warp::reject::not_found());
    }
    
    // Generate operation ID
    let operation_id = format!("zip_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis());
    
    // Initialize progress
    state.update_progress(&operation_id, crate::models::ZipProgress {
        current_file: "Scanning directory...".to_string(),
        processed_files: 0,
        total_files: 0,
        percentage: 0.0,
    });
    
    // Count files in background
    let op_id = operation_id.clone();
    let path_clone = full_path.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        let total = count_files_in_directory(&path_clone);
        state_clone.update_progress(&op_id, crate::models::ZipProgress {
            current_file: "Ready to start download...".to_string(),
            processed_files: 0,
            total_files: total,
            percentage: 0.0,
        });
    });
    
    Ok(warp::reply::json(&serde_json::json!({
        "success": true,
        "operationId": operation_id
    })))
}

pub async fn handle_download_folder(query: DownloadQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    let root_path = state.get_root_path();
    
    // Validate path
    let path = Path::new(&query.path);
    let mut full_path = root_path.clone();
    for component in path.components() {
        match component {
            std::path::Component::Normal(name) => full_path.push(name),
            _ => continue,
        }
    }
    
    if !full_path.starts_with(&root_path) || !full_path.is_dir() {
        return Err(warp::reject::not_found());
    }
    
    // Get operation ID
    let operation_id = match query.operation_id {
        Some(id) => id,
        None => format!("zip_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()),
    };
    
    // Get folder name for the filename
    let folder_name = match full_path.file_name() {
        Some(name) => name.to_string_lossy().to_string(),
        None => "folder".to_string(),
    };
    
    // Create temp file
    let temp_file = match NamedTempFile::new() {
        Ok(file) => file,
        Err(_) => return Err(warp::reject::custom(ZipCreationError)),
    };
    
    // Count files if needed
    let total_files = match state.get_progress(&operation_id) {
        Some(progress) if progress.total_files > 0 => progress.total_files,
        _ => {
            let count = count_files_in_directory(&full_path);
            state.update_progress(&operation_id, crate::models::ZipProgress {
                current_file: "Starting compression...".to_string(),
                processed_files: 0,
                total_files: count,
                percentage: 0.0,
            });
            count
        }
    };
    
    // Update progress for ZIP creation
    state.update_progress(&operation_id, crate::models::ZipProgress {
        current_file: "Creating ZIP file...".to_string(),
        processed_files: 0,
        total_files,
        percentage: 0.0,
    });
    
    let temp_path = temp_file.path().to_path_buf();
    
    // Create ZIP file using Rust implementation
    if let Err(_) = create_zip_archive(
        full_path.clone(), 
        full_path,
        temp_path.clone(),
        operation_id.clone(),
        state.clone()
    ).await {
        return Err(warp::reject::custom(ZipCreationError));
    }
    
    // Clean up progress tracking
    state.remove_progress(&operation_id);
    
    // Read ZIP file
    let mut file = match fs::File::open(&temp_path) {
        Ok(file) => file,
        Err(_) => return Err(warp::reject::custom(ZipCreationError)),
    };
    
    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() {
        return Err(warp::reject::custom(ZipCreationError));
    }
    
    // Return response with appropriate headers
    let filename = format!("{}.zip", folder_name);
    let mut response = warp::reply::Response::new(buffer.into());
    let headers = response.headers_mut();
    headers.insert(warp::http::header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    headers.insert(
        warp::http::header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename)).unwrap(),
    );
    headers.insert(
        "X-Operation-Id",
        HeaderValue::from_str(&operation_id).unwrap(),
    );
    
    Ok(response)
}
