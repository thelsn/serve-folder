mod models;
mod state;
mod handlers;
mod zip;
mod web;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::oneshot;
use warp::Filter;

use crate::state::ServerState;
use crate::handlers::{handle_list, handle_stop, handle_download_folder, handle_zip_progress, handle_zip_init, handle_upload, handle_rejection, same_origin_only};
use crate::web::serve_web_ui;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: serve_folder <directory>");
        std::process::exit(1);
    }

    // Explorer passes a drive root as "C:\" and the trailing backslash escapes the closing
    // quote, so it arrives as C:" (and a bare C: would mean the current dir on C:, not the
    // root). Quotes can't appear in Windows paths, so turn it back into the backslash.
    let mut arg = args[1].clone();
    if cfg!(windows) && arg.trim_end().ends_with('"') {
        arg.truncate(arg.trim_end().len() - 1);
        arg.push('\\');
    }
    let serve_path = PathBuf::from(arg);
    if !serve_path.is_dir() {
        eprintln!("Error: Provided path is not a directory");
        std::process::exit(1);
    }

    // Create shared state for server control
    let state = ServerState::new(serve_path.clone());

    // Create a channel for server shutdown
    let (tx, rx) = oneshot::channel::<()>();
    state.set_shutdown_tx(tx);

    // Create API routes
    let api_stop = warp::path!("api" / "stop")
        .and(warp::post())
        .and(same_origin_only())
        .and(warp::body::json())
        .and(state.with_state())
        .and_then(handle_stop);

    let api_list = warp::path!("api" / "list" / ..)
        .and(warp::query())
        .and(state.with_state())
        .and_then(handle_list);

    let api_download_folder = warp::path!("api" / "download" / "folder")
        .and(warp::get())
        .and(warp::query())
        .and(state.with_state())
        .and_then(handle_download_folder);

    let api_zip_progress = warp::path!("api" / "zip" / "progress")
        .and(warp::get())
        .and(warp::query())
        .and(state.with_state())
        .and_then(handle_zip_progress);

    let api_zip_init = warp::path!("api" / "zip" / "init")
        .and(warp::get())
        .and(warp::query())
        .and(state.with_state())
        .and_then(handle_zip_init);

    let api_upload = warp::path!("api" / "upload")
        .and(warp::post())
        .and(same_origin_only())
        // No size cap: files are streamed to disk, and whole folders can be uploaded at once
        .and(warp::multipart::form().max_length(None))
        .and(warp::query())
        .and(warp::header::optional::<String>("sec-fetch-mode"))
        .and(state.with_state())
        .and_then(handle_upload);

    // Serve web UI files
    let web_ui = warp::path("webui")
        .and(warp::get())
        .and(warp::path::tail())
        .and_then(serve_web_ui);

    // Redirect root to web UI. Not a permanent redirect: browsers cache those forever,
    // which would break whatever else is later run on port 8080.
    let root_redirect = warp::path::end()
        .and(warp::get())
        .map(|| warp::redirect::found(warp::http::Uri::from_static("/webui/")));

    // Create combined routes
    let routes = api_stop
        .or(api_list)
        .or(api_download_folder)
        .or(api_zip_progress)
        .or(api_zip_init)
        .or(api_upload)
        .or(web_ui)
        .or(root_redirect)
        .or(warp::fs::dir(serve_path))
        .recover(handle_rejection);

    let addr: SocketAddr = ([0, 0, 0, 0], 8080).into();

    // Run server with graceful shutdown
    let server = match warp::serve(routes)
        .try_bind_with_graceful_shutdown(addr, async {
            rx.await.ok();
            println!("Server shutting down");
        }) {
        Ok((_, server)) => server,
        Err(err) => {
            eprintln!("Error: could not listen on port 8080 (is another server already running?): {}", err);
            std::process::exit(1);
        }
    };

    println!("ServeOn8080 v{}", env!("CARGO_PKG_VERSION"));
    println!("Serving on http://127.0.0.1:8080 Visit this URL to access the web UI.");
    println!("Press Ctrl+C to stop the server");

    // Run the server
    server.await;
}
