use warp::{Reply, Rejection};

// Serve embedded web UI files
pub async fn serve_web_ui(path: warp::path::Tail) -> Result<impl Reply, Rejection> {
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
