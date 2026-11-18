use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};

use axum::{Json, Router, routing::get};
use local_ip_address::local_ip;
use sea_orm::DatabaseConnection;
use serde::Serialize;
use tokio::fs;

use crate::services::{files, qr, upload};

const PREFERRED_SERVER_PORT: u16 = 3000;
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
static SERVER_PORT: AtomicU16 = AtomicU16::new(0);

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) upload_dir: Arc<PathBuf>,
    pub(crate) db: DatabaseConnection,
}

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
        port: SERVER_PORT.load(Ordering::Acquire),
    })
}

pub(crate) fn port() -> Option<u16> {
    match SERVER_PORT.load(Ordering::Acquire) {
        0 => None,
        port => Some(port),
    }
}

async fn bind_listener(preferred_port: u16) -> Result<tokio::net::TcpListener, std::io::Error> {
    match tokio::net::TcpListener::bind(("0.0.0.0", preferred_port)).await {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let listener = tokio::net::TcpListener::bind(("0.0.0.0", 0)).await?;
            eprintln!(
                "Port {preferred_port} is busy; using port {} instead.",
                listener.local_addr()?.port()
            );
            Ok(listener)
        }
        Err(error) => Err(error),
    }
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/upload", axum::routing::post(upload::upload))
        .route("/download/{filename}", get(files::download))
        .route("/server-info", get(server_info))
        .route("/qr", get(qr::generate))
        .with_state(state)
}

pub(crate) async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = bind_listener(PREFERRED_SERVER_PORT).await?;
    let port = listener.local_addr()?.port();

    let upload_dir = PathBuf::from("uploads");
    fs::create_dir_all(&upload_dir).await?;

    let db = files::connect().await?;
    files::reconcile(&upload_dir, &db).await;

    let cleanup_dir = upload_dir.clone();
    let cleanup_db = db.clone();
    tokio::spawn(async move {
        files::delete_expired(&cleanup_dir, &cleanup_db).await;
        let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            files::delete_expired(&cleanup_dir, &cleanup_db).await;
        }
    });

    let state = AppState {
        upload_dir: Arc::new(upload_dir),
        db,
    };
    SERVER_PORT.store(port, Ordering::Release);
    let ip = local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    println!("File sharing server listening at http://{ip}:{port}");

    axum::serve(listener, app(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::bind_listener;

    #[tokio::test]
    async fn uses_an_available_port_when_preferred_port_is_busy() {
        let blocker = tokio::net::TcpListener::bind(("0.0.0.0", 0))
            .await
            .expect("Could not reserve a port for the test");
        let blocked_port = blocker
            .local_addr()
            .expect("Could not read the reserved port")
            .port();

        let listener = bind_listener(blocked_port)
            .await
            .expect("Could not bind fallback port");

        assert_ne!(
            listener
                .local_addr()
                .expect("Could not read listener port")
                .port(),
            blocked_port
        );
    }
}
