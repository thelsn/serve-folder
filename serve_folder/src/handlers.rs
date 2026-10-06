use std::path::{Component, Path, PathBuf};
use std::fs;
use std::io;
use warp::{Filter, Reply, Rejection, http::{HeaderValue, StatusCode, Uri}, multipart::{FormData, Part}};
use futures_util::TryStreamExt;
use tokio::io::AsyncWriteExt;

use crate::models::{FileEntry, DirResponse, StopRequest, DownloadQuery, ProgressQuery, ZipProgress, CrossSiteRequest};
use crate::state::ServerState;
use crate::zip::stream_zip_archive;

// Maps a client-supplied relative path onto the served folder. Only plain segments are
// kept, so "..", drive prefixes and absolute paths can't escape the root.
fn resolve_path(root_path: &Path, relative_path: &str) -> PathBuf {
    let mut full_path = root_path.to_path_buf();
    for component in Path::new(relative_path).components() {
        if let Component::Normal(name) = component {
            full_path.push(name);
        }
    }
    full_path
}

// Paths sent to the browser always use '/', which the UI and the static file route expect
// (on Windows the OS separator is '\', which the static file route rejects)
fn to_url_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Rejects state-changing requests sent by other websites (CSRF), e.g. a page in another
/// tab silently stopping the server or dropping files into the shared folder. Browsers
/// send Sec-Fetch-Site; older ones are checked via Origin. Requests with neither header
/// come from non-browser clients such as curl and are allowed.
pub fn same_origin_only() -> impl Filter<Extract = (), Error = Rejection> + Clone {
    warp::header::optional::<String>("sec-fetch-site")
        .and(warp::header::optional::<String>("origin"))
        .and(warp::header::optional::<String>("host"))
        .and_then(|site: Option<String>, origin: Option<String>, host: Option<String>| async move {
            let allowed = match (site, origin) {
                // "none" means the user initiated it directly, e.g. sharing to the installed app
                (Some(site), _) => site == "same-origin" || site == "none",
                (None, Some(origin)) => match (origin.split_once("://"), host) {
                    (Some((_, origin_host)), Some(host)) => origin_host.eq_ignore_ascii_case(&host),
                    _ => false,
                },
                (None, None) => true,
            };
            if allowed {
                Ok(())
            } else {
                Err(warp::reject::custom(CrossSiteRequest))
            }
        })
        .untuple_one()
}

pub async fn handle_rejection(err: Rejection) -> Result<impl Reply, Rejection> {
    if err.find::<CrossSiteRequest>().is_some() {
        return Ok(warp::reply::with_status("Cross-site requests are not allowed", StatusCode::FORBIDDEN));
    }
    Err(err)
}

fn list_directory(root_path: &Path, target_path: &Path) -> io::Result<DirResponse> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(target_path)?.flatten() {
        let path = entry.path();
        let metadata = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };

        // Get relative path from root
        let rel_path = path.strip_prefix(root_path).unwrap_or(&path);

        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: to_url_path(rel_path),
            is_dir: metadata.is_dir(),
            size: if metadata.is_file() { metadata.len() } else { 0 },
        });
    }

    // Sort entries: directories first, then files
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let rel_current = target_path.strip_prefix(root_path).unwrap_or(Path::new(""));
    Ok(DirResponse {
        current_path: to_url_path(rel_current),
        entries,
    })
}

pub async fn handle_list(query: DownloadQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    let root_path = state.get_root_path();
    let target_path = resolve_path(&root_path, &query.path);

    // Directory reads can be slow (large folders, network drives), so keep them off the async workers
    let listing = tokio::task::spawn_blocking(move || list_directory(&root_path, &target_path)).await;

    match listing {
        Ok(Ok(response)) => Ok(warp::reply::json(&response)),
        // Missing folder, a file, or no permission
        _ => Err(warp::reject::not_found()),
    }
}

pub async fn handle_stop(stop_req: StopRequest, state: ServerState) -> Result<impl Reply, Rejection> {
    if !stop_req.confirm {
        return Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "message": "Stop request was not confirmed"
        })));
    }

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
    // Unknown ids are a 404 so the client can tell "finished/expired" apart from "starting"
    match state.get_progress(&query.id) {
        Some(progress) => Ok(warp::reply::json(&progress)),
        None => Err(warp::reject::not_found()),
    }
}

pub async fn handle_zip_init(query: DownloadQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    let full_path = resolve_path(&state.get_root_path(), &query.path);
    if !full_path.is_dir() {
        return Err(warp::reject::not_found());
    }

    let operation_id = state.new_operation_id();
    state.update_progress(&operation_id, ZipProgress {
        current_file: "Starting...".to_string(),
        ..Default::default()
    });

    Ok(warp::reply::json(&serde_json::json!({
        "success": true,
        "operationId": operation_id
    })))
}

// The id is used as a map key and echoed back in a header, so only accept simple ids
fn is_valid_operation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// An ASCII fallback name plus the RFC 5987 UTF-8 form, so any folder name yields a valid header
fn content_disposition(filename: &str) -> HeaderValue {
    let fallback: String = filename
        .chars()
        .map(|c| if c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\') { c } else { '_' })
        .collect();
    let encoded: String = filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect();
    HeaderValue::from_str(&format!("attachment; filename=\"{}\"; filename*=UTF-8''{}", fallback, encoded))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

pub async fn handle_download_folder(query: DownloadQuery, state: ServerState) -> Result<impl Reply, Rejection> {
    let full_path = resolve_path(&state.get_root_path(), &query.path);
    let operation_id = query
        .operation_id
        .filter(|id| is_valid_operation_id(id))
        .unwrap_or_else(|| state.new_operation_id());

    if !full_path.is_dir() {
        state.update_progress(&operation_id, ZipProgress {
            current_file: "Folder not found".to_string(),
            failed: true,
            ..Default::default()
        });
        return Err(warp::reject::not_found());
    }

    // Get folder name for the filename
    let folder_name = match full_path.file_name() {
        Some(name) => name.to_string_lossy().to_string(),
        None => "folder".to_string(),
    };

    // The zip is sent as it's built, so its size isn't known up front (no Content-Length)
    let stream = stream_zip_archive(full_path, operation_id.clone(), state);
    let body = warp::hyper::Body::wrap_stream(stream);
    let mut response = warp::reply::Response::new(body);
    let headers = response.headers_mut();
    headers.insert(warp::http::header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    headers.insert(
        warp::http::header::CONTENT_DISPOSITION,
        content_disposition(&format!("{}.zip", folder_name)),
    );
    if let Ok(value) = HeaderValue::from_str(&operation_id) {
        headers.insert("X-Operation-Id", value);
    }

    Ok(response)
}

fn upload_error(status: StatusCode, message: String, uploaded_files: &[String]) -> warp::reply::Response {
    warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "success": false,
            "message": message,
            "uploaded": uploaded_files,
            "count": uploaded_files.len()
        })),
        status,
    )
    .into_response()
}

pub async fn handle_upload(
    form: FormData,
    query: DownloadQuery,
    fetch_mode: Option<String>,
    state: ServerState,
) -> Result<warp::reply::Response, Rejection> {
    let target_dir = resolve_path(&state.get_root_path(), &query.path);

    // Create directory if it doesn't exist (fails if the path is an existing file)
    if let Err(err) = tokio::fs::create_dir_all(&target_dir).await {
        return Ok(upload_error(StatusCode::BAD_REQUEST, format!("Cannot use upload folder: {}", err), &[]));
    }

    // Process uploaded files
    let mut uploaded_files = Vec::new();
    let mut parts = form;

    loop {
        let part = match parts.try_next().await {
            Ok(Some(part)) => part,
            Ok(None) => break,
            Err(err) => {
                return Ok(upload_error(StatusCode::BAD_REQUEST, format!("Upload interrupted: {}", err), &uploaded_files));
            }
        };

        let (folders, filename) = match part.filename() {
            Some(name) => sanitize_upload_path(name),
            None => continue,
        };

        // Folder uploads send each file's path within the chosen folder; recreate it
        let dir = folders.iter().fold(target_dir.clone(), |dir, folder| dir.join(folder));
        if let Err(err) = tokio::fs::create_dir_all(&dir).await {
            return Ok(upload_error(
                StatusCode::BAD_REQUEST,
                format!("Cannot create folder {}: {}", folders.join("/"), err),
                &uploaded_files,
            ));
        }

        match save_part(part, &dir, &filename).await {
            Ok(saved_name) => uploaded_files.push(
                folders.iter().map(String::as_str).chain([saved_name.as_str()]).collect::<Vec<_>>().join("/"),
            ),
            Err(err) => {
                return Ok(upload_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to save {}: {}", filename, err),
                    &uploaded_files,
                ));
            }
        }
    }

    // Files shared to the installed app arrive as a page navigation, so send the user back to the UI
    if fetch_mode.as_deref() == Some("navigate") {
        return Ok(warp::redirect::see_other(Uri::from_static("/webui/")).into_response());
    }

    Ok(warp::reply::json(&serde_json::json!({
        "success": true,
        "uploaded": uploaded_files,
        "count": uploaded_files.len()
    }))
    .into_response())
}

// Streams one uploaded file to disk, removing the partial file if the upload breaks off.
// Returns the name it was saved under.
async fn save_part(part: Part, dir: &Path, filename: &str) -> io::Result<String> {
    let (saved_name, file) = create_unique_file(dir, filename).await?;

    let mut writer = tokio::io::BufWriter::with_capacity(1024 * 1024, file);
    let mut stream = part.stream();
    let result = async {
        while let Some(mut chunk) = stream
            .try_next()
            .await
            .map_err(io::Error::other)?
        {
            writer.write_all_buf(&mut chunk).await?;
        }
        writer.flush().await
    }
    .await;

    if let Err(err) = result {
        drop(writer);
        let _ = tokio::fs::remove_file(dir.join(&saved_name)).await;
        return Err(err);
    }
    Ok(saved_name)
}

// Never overwrites: "name.ext" becomes "name (1).ext", "name (2).ext", ... if taken
async fn create_unique_file(dir: &Path, filename: &str) -> io::Result<(String, tokio::fs::File)> {
    let as_path = Path::new(filename);
    let stem = as_path.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let extension = as_path.extension().and_then(|s| s.to_str());

    for n in 0..10_000 {
        let candidate = match (n, extension) {
            (0, _) => filename.to_string(),
            (_, Some(ext)) => format!("{} ({}).{}", stem, n, ext),
            (_, None) => format!("{} ({})", stem, n),
        };
        let path = dir.join(&candidate);
        // Also catches folders with the same name, which create_new may not report as AlreadyExists
        if tokio::fs::symlink_metadata(&path).await.is_ok() {
            continue;
        }
        match tokio::fs::OpenOptions::new().write(true).create_new(true).open(&path).await {
            Ok(file) => return Ok((candidate, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with this name"))
}

// Splits a client-supplied file name into safe folder names plus a file name. Folder
// uploads send paths such as "Holiday/day 1/photo.jpg", which keep their structure.
fn sanitize_upload_path(filename: &str) -> (Vec<String>, String) {
    // Old browsers sent the full client-side path ("C:\fakepath\photo.jpg"); keep the name
    let filename = if filename.contains('/') { filename } else { filename.rsplit('\\').next().unwrap_or("") };

    let mut segments: Vec<&str> = filename.split('/').collect();
    let name = segments.pop().and_then(sanitize_segment).unwrap_or_else(|| {
        format!(
            "upload_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        )
    });
    // Empty, "." and ".." segments are dropped, so the path can't leave the upload folder
    let folders = segments.into_iter().filter_map(sanitize_segment).collect();
    (folders, name)
}

// Reduces one path segment to a safe file or folder name. Only characters that are invalid
// or dangerous in file names are replaced, so normal names are kept as-is.
fn sanitize_segment(segment: &str) -> Option<String> {
    // ':' would write to an NTFS alternate data stream instead of a visible file
    let name: String = segment
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\') { '_' } else { c })
        .collect();

    // Windows drops trailing dots and spaces; this also turns "." and ".." into ""
    let mut name = name.trim().trim_end_matches(['.', ' ']).to_string();

    // Device names like CON or NUL.txt would write to the device rather than a file
    if cfg!(windows) {
        let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
        let is_reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        if is_reserved {
            name = format!("_{}", name);
        }
    }

    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload_path(filename: &str) -> (Vec<String>, String) {
        sanitize_upload_path(filename)
    }

    #[test]
    fn keeps_folder_structure_of_folder_uploads() {
        assert_eq!(upload_path("Holiday/day 1/photo (1).jpg"), (vec!["Holiday".into(), "day 1".into()], "photo (1).jpg".into()));
        assert_eq!(upload_path("report.pdf"), (vec![], "report.pdf".into()));
    }

    #[test]
    fn upload_paths_cannot_escape_the_folder() {
        assert_eq!(upload_path("../../evil.txt"), (vec![], "evil.txt".into()));
        assert_eq!(upload_path("a/./../b/c.txt"), (vec!["a".into(), "b".into()], "c.txt".into()));
        assert_eq!(upload_path("/abs/olute.txt"), (vec!["abs".into()], "olute.txt".into()));
        assert_eq!(upload_path(r"C:\fakepath\photo.jpg"), (vec![], "photo.jpg".into()));
        assert_eq!(upload_path(r"dir\x/a:b.txt"), (vec!["dir_x".into()], "a_b.txt".into()));
    }

    #[test]
    fn unusable_names_get_a_fallback() {
        let (folders, name) = upload_path("..");
        assert!(folders.is_empty());
        assert!(name.starts_with("upload_"), "{name}");
        assert!(upload_path("folder/").1.starts_with("upload_"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_device_names_are_escaped() {
        assert_eq!(upload_path("CON.txt").1, "_CON.txt");
        assert_eq!(upload_path("nul/com1").0, vec!["_nul".to_string()]);
        assert_eq!(upload_path("nul/com1").1, "_com1");
        assert_eq!(upload_path("console.txt").1, "console.txt");
    }
}
