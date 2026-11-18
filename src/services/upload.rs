use axum::{
    Json,
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use robius_file_picker::LocalFile;
use serde::{Deserialize, Serialize};
use std::{error::Error, io};
use tokio::fs;
use uuid::Uuid;

use crate::{server::AppState, services::files};

const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;
const FILE_LIFETIME_SECONDS: i64 = 60 * 60;

#[derive(Deserialize, Serialize)]
struct UploadResponse {
    success: bool,
    id: String,
    download_url: String,
}

#[derive(Deserialize)]
struct ServerInfo {
    ip: String,
    port: u16,
}

pub(crate) async fn send_selected_file(
    file: LocalFile,
    port: u16,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let file_name = file
        .display_name()
        .map(str::to_owned)
        .or_else(|| {
            file.path()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "upload.bin".to_string());
    let data = fs::read(file.path()).await?;
    if data.is_empty() {
        return Err(io::Error::other("Selected file is empty").into());
    }
    if data.len() > MAX_FILE_SIZE {
        return Err(io::Error::other("Maximum file size is 10 MB").into());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let server_info = get_server_info(&client, port).await?;

    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(data).file_name(file_name),
    );
    let response = client
        .post(format!("http://127.0.0.1:{port}/upload"))
        .multipart(form)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(io::Error::other(format!("Upload failed ({status}): {body}")).into());
    }

    let result = response.json::<UploadResponse>().await?;
    if !result.success || !result.download_url.starts_with("/download/") {
        return Err(io::Error::other("Server returned an invalid upload response").into());
    }

    Ok(format!(
        "http://{}:{}{}",
        server_info.ip, server_info.port, result.download_url
    ))
}

async fn get_server_info(
    client: &reqwest::Client,
    port: u16,
) -> Result<ServerInfo, Box<dyn Error + Send + Sync>> {
    let url = format!("http://127.0.0.1:{port}/server-info");
    let mut last_error = "server did not respond".to_string();

    for _ in 0..20 {
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => match response.json().await {
                Ok(server_info) => return Ok(server_info),
                Err(error) => last_error = error.to_string(),
            },
            Ok(response) => last_error = format!("server returned {}", response.status()),
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    Err(io::Error::other(format!("File sharing server is unavailable: {last_error}")).into())
}

pub(crate) async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> Response {
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
                    if data.len() + chunk.len() > MAX_FILE_SIZE {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "Maximum file size is 10 MB")
                            .into_response();
                    }
                    data.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => {
                    eprintln!("Could not read upload: {error}");
                    return (StatusCode::BAD_REQUEST, "Failed to read upload").into_response();
                }
            }
        }
        if data.is_empty() {
            return (StatusCode::BAD_REQUEST, "Empty file").into_response();
        }

        let filename = format!("{}.{}", Uuid::new_v4(), extension);
        let path = state.upload_dir.join(&filename);
        if let Err(error) = fs::write(&path, data).await {
            eprintln!("Could not save uploaded file: {error}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not save file").into_response();
        }

        let uploaded_at = files::unix_timestamp();
        if let Err(error) = files::track_upload(
            filename.clone(),
            uploaded_at,
            uploaded_at + FILE_LIFETIME_SECONDS,
            &state.db,
        )
        .await
        {
            let _ = fs::remove_file(&path).await;
            eprintln!("Could not track uploaded file: {error}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not track uploaded file",
            )
                .into_response();
        }

        return Json(UploadResponse {
            success: true,
            id: filename.clone(),
            download_url: format!("/download/{filename}"),
        })
        .into_response();
    }

    (StatusCode::BAD_REQUEST, "No file uploaded").into_response()
}
