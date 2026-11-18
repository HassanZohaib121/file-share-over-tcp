use axum::{
    Json, Router,
    extract::{Multipart, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::fs;
use uuid::Uuid;

const MAX_SIZE: usize = 10 * 1024 * 1024; // 10 MB
const FILE_LIFETIME: Duration = Duration::from_secs(60 * 60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

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

fn is_expired(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= FILE_LIFETIME)
}

async fn delete_expired_files(upload_dir: &std::path::Path) {
    let mut entries = match fs::read_dir(upload_dir).await {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Could not read uploads directory: {}", error);
            return;
        }
    };

    let now = SystemTime::now();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path).await else {
            continue;
        };

        if metadata.is_file() {
            let Ok(modified) = metadata.modified() else {
                continue;
            };

            if is_expired(modified, now) {
                if let Err(error) = fs::remove_file(&path).await {
                    eprintln!(
                        "Could not delete expired file {}: {}",
                        path.display(),
                        error
                    );
                } else {
                    println!("Deleted expired file: {}", path.display());
                }
            }
        }
    }
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

// Upload file
async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    while let Ok(Some(mut field)) = multipart.next_field().await {
        if !matches!(field.name(), Some("file") | Some("image")) {
            continue;
        }

        let extension = field
            .file_name()
            .and_then(|name| name.rsplit('.').next())
            .map(str::trim)
            .filter(|extension| {
                !extension.is_empty()
                    && extension.len() <= 20
                    && extension
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            })
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "bin".to_string());

        let mut data = Vec::new();

        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if data.len() + chunk.len() > MAX_SIZE {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "Maximum file size is 10 MB")
                            .into_response();
                    }

                    data.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => {
                    return (StatusCode::BAD_REQUEST, "Failed to read upload").into_response();
                }
            }
        }

        if data.is_empty() {
            return (StatusCode::BAD_REQUEST, "Empty file").into_response();
        }

        let id = Uuid::new_v4().to_string();
        let filename = format!("{}.{}", id, extension);
        let path = state.upload_dir.join(&filename);

        if fs::write(path, data).await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not save file").into_response();
        }

        let result = UploadResponse {
            success: true,
            id: filename.clone(),
            download_url: format!("/download/{}", filename),
        };

        return Json(result).into_response();
    }

    (StatusCode::BAD_REQUEST, "No file uploaded").into_response()
}

// Download file
async fn download(State(state): State<AppState>, Path(filename): Path<String>) -> Response {
    let Some((id, extension)) = filename.rsplit_once('.') else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    if Uuid::parse_str(id).is_err()
        || extension.is_empty()
        || extension.len() > 20
        || !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = state.upload_dir.join(&filename);

    match fs::metadata(&path).await {
        Ok(metadata) if metadata.is_file() => match metadata.modified() {
            Ok(modified) if is_expired(modified, SystemTime::now()) => {
                let _ = fs::remove_file(&path).await;
                return StatusCode::NOT_FOUND.into_response();
            }
            Ok(_) => {}
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        _ => return StatusCode::NOT_FOUND.into_response(),
    }

    let data = match fs::read(path).await {
        Ok(data) => data,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let mut response = Response::new(data.into());

    *response.status_mut() = StatusCode::OK;

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
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

    let cleanup_dir = Arc::clone(&state.upload_dir);
    tokio::spawn(async move {
        delete_expired_files(&cleanup_dir).await;

        let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            delete_expired_files(&cleanup_dir).await;
        }
    });

    let app = Router::new()
        .route("/", get(home))
        .route("/upload", post(upload))
        .route("/download/{filename}", get(download))
        .route("/server-info", axum::routing::get(server_info))
        .with_state(state);

    let ip = get_server_ip();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("================================");
    println!("File Share Server Started");
    println!("Open on other devices:");
    println!("http://{}:3000", ip);
    println!("================================");

    // axum::serve(listener, app).await.unwrap();

    // let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
    //     .await
    //     .expect("Could not bind port");

    // println!("Server running at http://localhost:3000");

    axum::serve(listener, app).await.expect("Server error");
}
