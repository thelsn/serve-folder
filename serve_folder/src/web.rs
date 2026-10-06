use warp::{Reply, Rejection};

// Serve embedded web UI files
pub async fn serve_web_ui(path: warp::path::Tail) -> Result<impl Reply, Rejection> {
    let path = path.as_str();
    let content_type = match path {
        "" | "index.html" => ("text/html; charset=utf-8", include_str!("../web/index.html")),
        "style.css" => ("text/css; charset=utf-8", include_str!("../web/style.css")),
        "styles.css" => ("text/css; charset=utf-8", include_str!("../web/styles.css")),
        "script.js" => ("application/javascript; charset=utf-8", include_str!("../web/script.js")),
        "service-worker.js" => ("application/javascript; charset=utf-8", include_str!("../web/service-worker.js")),
        "manifest.json" => ("application/manifest+json", include_str!("../web/manifest.json")),
        "icon.svg" => ("image/svg+xml", include_str!("../web/icon.svg")),
        _ => return Err(warp::reject::not_found()),
    };

    // The UI is compiled into the binary, so have browsers revalidate to pick up new builds
    Ok(warp::reply::with_header(
        warp::reply::with_header(content_type.1, "content-type", content_type.0),
        "cache-control",
        "no-cache",
    ))
}
