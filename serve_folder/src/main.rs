use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use warp::{Filter, Reply, Rejection};
use serde::{Serialize, Deserialize};
use std::fs;

#[derive(Serialize)]
struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
struct DirResponse {
    current_path: String,
    entries: Vec<FileEntry>,
}

#[derive(Deserialize)]
struct StopRequest {
    confirm: bool,
}

struct ServerState {
    shutdown_tx: Option<oneshot::Sender<()>>,
    root_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: serve_folder <directory>");
        std::process::exit(1);
    }

    let serve_path = PathBuf::from(&args[1]);
    if !serve_path.is_dir() {
        eprintln!("Error: Provided path is not a directory");
        std::process::exit(1);
    }

    // Create shared state for server control
    let state = Arc::new(Mutex::new(ServerState {
        shutdown_tx: None,
        root_path: serve_path.clone(),
    }));

    // Create a channel for server shutdown
    let (tx, rx) = oneshot::channel::<()>();
    state.lock().unwrap().shutdown_tx = Some(tx);

    // Create API routes
    let state_clone = Arc::clone(&state);
    let api_stop = warp::path!("api" / "stop")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state_clone))
        .and_then(handle_stop);

    let state_clone = Arc::clone(&state);
    let api_list = warp::path!("api" / "list" / ..)
        .and(warp::query::<ListQuery>())
        .and(with_state(state_clone))
        .and_then(handle_list);

    // Serve web UI files (embedded in the binary)
    let web_ui = warp::path("webui")
        .and(warp::get())
        .and(warp::path::tail())
        .and_then(serve_web_ui);

    // Redirect root to web UI
    let root_redirect = warp::path::end()
        .and(warp::get())
        .map(|| warp::redirect(warp::http::Uri::from_static("/webui")));

    // Create combined routes
    let routes = api_stop
        .or(api_list)
        .or(web_ui)
        .or(root_redirect)
        .or(warp::fs::dir(serve_path));

    let addr: SocketAddr = ([0, 0, 0, 0], 8080).into();
    println!("Serving on http://127.0.0.1:8080 Visit this URL to access the web UI.");
    println!("Press Ctrl+C to stop the server");

    // Run server with graceful shutdown
    let (_, server) = warp::serve(routes)
        .bind_with_graceful_shutdown(addr, async {
            rx.await.ok();
            println!("Server shutting down");
        });

    // Run the server
    server.await;
}

fn with_state(state: Arc<Mutex<ServerState>>) -> impl Filter<Extract = (Arc<Mutex<ServerState>>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || Arc::clone(&state))
}

#[derive(Deserialize)]
struct ListQuery {
    path: Option<String>,
}

async fn handle_list(query: ListQuery, state: Arc<Mutex<ServerState>>) -> Result<impl Reply, Rejection> {
    let state_guard = state.lock().unwrap();
    let root_path = &state_guard.root_path;
    
    // Determine which path to list (default to root if not specified)
    let relative_path = query.path.unwrap_or_default();
    let target_path = if relative_path.is_empty() {
        root_path.clone()
    } else {
        // Sanitize and validate the path to prevent directory traversal attacks
        let path = Path::new(&relative_path);
        let mut full_path = root_path.clone();
        for component in path.components() {
            match component {
                std::path::Component::Normal(name) => full_path.push(name),
                _ => continue, // Skip other components for security
            }
        }
        
        // Verify the path is within the root directory
        if !full_path.starts_with(root_path) {
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
                    let rel_path = path.strip_prefix(root_path).unwrap_or(&path);
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
    
    let rel_current = target_path.strip_prefix(root_path).unwrap_or(Path::new(""));
    let current_path = rel_current.to_string_lossy().to_string();
    
    let response = DirResponse {
        current_path,
        entries,
    };
    
    Ok(warp::reply::json(&response))
}

async fn handle_stop(stop_req: StopRequest, state: Arc<Mutex<ServerState>>) -> Result<impl Reply, Rejection> {
    if stop_req.confirm {
        // Take the sender out to avoid multiple stops
        let tx = state.lock().unwrap().shutdown_tx.take();
        
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
    }
    
    Ok(warp::reply::json(&serde_json::json!({
        "success": false,
        "message": "Failed to stop server"
    })))
}

async fn serve_web_ui(path: warp::path::Tail) -> Result<impl Reply, Rejection> {
    let path = path.as_str();
    let content_type = match path {
        "" | "index.html" => ("text/html", include_str!("../web/index.html")),
        "style.css" => ("text/css", include_str!("../web/style.css")),
        "script.js" => ("application/javascript", include_str!("../web/script.js")),
        _ => return Err(warp::reject::not_found()),
    };
    
    Ok(warp::reply::with_header(
        content_type.1,
        "content-type",
        content_type.0,
    ))
}
