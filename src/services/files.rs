use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    QueryFilter, Set,
};
use tokio::fs;

use crate::server::AppState;

const FILE_LIFETIME: Duration = Duration::from_secs(60 * 60);

mod entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "uploaded_files")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub filename: String,
        pub uploaded_at: i64,
        pub delete_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub(super) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) async fn track_upload(
    filename: String,
    uploaded_at: i64,
    delete_at: i64,
    db: &DatabaseConnection,
) -> Result<(), sea_orm::DbErr> {
    entity::ActiveModel {
        filename: Set(filename),
        uploaded_at: Set(uploaded_at),
        delete_at: Set(delete_at),
    }
    .insert(db)
    .await
    .map(|_| ())
}

pub async fn connect() -> Result<DatabaseConnection, sea_orm::DbErr> {
    let db = Database::connect("sqlite:file_share.db?mode=rwc").await?;
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS uploaded_files (
            filename TEXT PRIMARY KEY NOT NULL,
            uploaded_at INTEGER NOT NULL,
            delete_at INTEGER NOT NULL
        )",
    )
    .await?;
    Ok(db)
}

pub async fn reconcile(upload_dir: &Path, db: &DatabaseConnection) {
    let mut entries = match fs::read_dir(upload_dir).await {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("Could not read uploads directory: {error}");
            return;
        }
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path).await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }

        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if entity::Entity::find_by_id(filename)
            .one(db)
            .await
            .ok()
            .flatten()
            .is_some()
        {
            continue;
        }

        let uploaded_at = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_else(unix_timestamp);
        let record = entity::ActiveModel {
            filename: Set(filename.to_string()),
            uploaded_at: Set(uploaded_at),
            delete_at: Set(uploaded_at + FILE_LIFETIME.as_secs() as i64),
        };

        if let Err(error) = record.insert(db).await {
            eprintln!("Could not track existing file {filename}: {error}");
        }
    }
}

pub async fn delete_expired(upload_dir: &Path, db: &DatabaseConnection) {
    let expired = match entity::Entity::find()
        .filter(entity::Column::DeleteAt.lt(unix_timestamp()))
        .all(db)
        .await
    {
        Ok(files) => files,
        Err(error) => {
            eprintln!("Could not query expired files: {error}");
            return;
        }
    };

    for file in expired {
        let path = upload_dir.join(&file.filename);
        match fs::remove_file(&path).await {
            Ok(()) => println!("Deleted expired file: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!("Could not delete expired file {}: {error}", path.display());
                continue;
            }
        }

        if let Err(error) = entity::Entity::delete_by_id(file.filename).exec(db).await {
            eprintln!("Could not delete file record: {error}");
        }
    }
}

pub(crate) async fn download(
    State(state): State<AppState>,
    AxumPath(filename): AxumPath<String>,
) -> Response {
    let Some((id, extension)) = filename.rsplit_once('.') else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if uuid::Uuid::parse_str(id).is_err()
        || extension.is_empty()
        || extension.len() > 20
        || !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let path = state.upload_dir.join(&filename);
    let record = match entity::Entity::find_by_id(&filename).one(&state.db).await {
        Ok(Some(record)) => record,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            eprintln!("Could not query file record: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if record.delete_at <= unix_timestamp() {
        let _ = fs::remove_file(&path).await;
        let _ = entity::Entity::delete_by_id(&filename)
            .exec(&state.db)
            .await;
        return StatusCode::NOT_FOUND.into_response();
    }

    if !fs::metadata(&path)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        let _ = entity::Entity::delete_by_id(&filename)
            .exec(&state.db)
            .await;
        return StatusCode::NOT_FOUND.into_response();
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
