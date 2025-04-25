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
use crate::handlers::{handle_list, handle_stop, handle_download_folder, handle_zip_progress, handle_zip_init};
use crate::web::serve_web_ui;

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
    let state = ServerState::new(serve_path.clone());

    // Create a channel for server shutdown
    let (tx, rx) = oneshot::channel::<()>();
    state.set_shutdown_tx(tx);

    // Create API routes
    let api_stop = warp::path!("api" / "stop")
        .and(warp::post())
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

    // Serve web UI files
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
        .or(api_download_folder)
        .or(api_zip_progress)
        .or(api_zip_init)
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
