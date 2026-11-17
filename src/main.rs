
use axum::{
    extract::{Multipart, Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tokio::fs;
use uuid::Uuid;

const MAX_SIZE: usize = 10 * 1024 * 1024; // 10 MB

#[derive(Clone)]
struct AppState {
    upload_dir: Arc<PathBuf>,
}

#[derive(Serialize)]
struct UploadResponse {
    success: bool,
    id: String,
    download_url: String,
}

async fn home() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

const SERVER_PORT: u16 = 3000;

#[derive(Serialize)]
struct ServerInfo {
    ip: String,
    port: u16,
}

async fn server_info() -> Json<ServerInfo> {
    let ip = match local_ip() {
        Ok(ip) if ip.is_ipv4() => ip.to_string(),
        _ => "127.0.0.1".to_string(),
    };

    Json(ServerInfo {
        ip,
        port: SERVER_PORT,
    })
}

use local_ip_address::local_ip;

fn get_server_ip() -> String {
    local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

// Upload image
async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    while let Ok(Some(mut field)) = multipart.next_field().await {
        if field.name() != Some("image") {
            continue;
        }

        let content_type = field
            .content_type()
            .unwrap_or("")
            .to_string();

        let extension = match content_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/bmp" => "bmp",
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Only image files are allowed",
                ).into_response();
            }
        };

        let mut data = Vec::new();

        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if data.len() + chunk.len() > MAX_SIZE {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "Maximum image size is 10 MB",
                        ).into_response();
                    }

                    data.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        "Failed to read upload",
                    ).into_response();
                }
            }
        }

        if data.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                "Empty image",
            ).into_response();
        }

        let id = Uuid::new_v4().to_string();
        let filename = format!("{}.{}", id, extension);
        let path = state.upload_dir.join(&filename);

        if fs::write(path, data).await.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not save image",
            ).into_response();
        }

        let result = UploadResponse {
            success: true,
            id: filename.clone(),
            download_url: format!("/download/{}", filename),
        };

        return Json(result).into_response();
    }

    (
        StatusCode::BAD_REQUEST,
        "No image uploaded",
    ).into_response()
}

// Download image
async fn download(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    let allowed = ["jpg", "png", "gif", "webp", "bmp"];

    let Some((id, extension)) = filename.rsplit_once('.') else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    if Uuid::parse_str(id).is_err() || !allowed.contains(&extension) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = state.upload_dir.join(&filename);

    let data = match fs::read(path).await {
        Ok(data) => data,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let mime = match extension {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    };

    let mut response = Response::new(data.into());

    *response.status_mut() = StatusCode::OK;

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(mime),
    );

    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment"),
    );

    response
}

#[tokio::main]
async fn main() {
    let upload_dir = PathBuf::from("uploads");

    fs::create_dir_all(&upload_dir)
        .await
        .expect("Could not create uploads directory");

    let state = AppState {
        upload_dir: Arc::new(upload_dir),
    };

    let app = Router::new()
        .route("/", get(home))
        .route("/upload", post(upload))
        .route("/download/{filename}", get(download))
        .route("/server-info", axum::routing::get(server_info))
        .with_state(state);

    let ip = get_server_ip();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    println!("================================");
    println!("Image Share Server Started");
    println!("Open on other devices:");
    println!("http://{}:3000", ip);
    println!("================================");

// axum::serve(listener, app).await.unwrap();

    // let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
    //     .await
    //     .expect("Could not bind port");

    // println!("Server running at http://localhost:3000");

    axum::serve(listener, app)
        .await
        .expect("Server error");
}
