slint::include_modules!();

mod clipboard;
mod server;
pub mod services;

fn post_to_ui(action: impl FnOnce() + Send + 'static) {
    if let Err(error) = slint::invoke_from_event_loop(action) {
        eprintln!("Could not update the UI: {error}");
    }
}

pub fn run() {
    start_server();
    run_rustdrop();
}

fn start_server() {
    std::thread::Builder::new()
        .name("rustdrop-server".to_string())
        .spawn(|| {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("Could not start server runtime: {error}");
                    return;
                }
            };

            if let Err(error) = runtime.block_on(server::run()) {
                eprintln!("File sharing server failed: {error}");
            }
        })
        .expect("Could not start file sharing server thread");
}

fn run_rustdrop() {
    let app = MainWindow::new().expect("Failed to initialize RustDrop");
    let selected_file =
        std::sync::Arc::new(std::sync::Mutex::new(None::<robius_file_picker::LocalFile>));

    let weak = app.as_weak();
    let selected_file_for_picker = std::sync::Arc::clone(&selected_file);

    app.on_select_file_requested(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status_text("Opening file picker...".into());

            let weak = weak.clone();
            let selected_file = std::sync::Arc::clone(&selected_file_for_picker);
            let picker_result = robius_file_picker::FileDialog::new()
                .set_title("Select a file")
                .pick_file(move |result| {
                    let local_file = match result {
                        Ok(Some(file)) => match file.into_local_file() {
                            Ok(file) => file,
                            Err(error) => {
                                let message = format!("Could not open selected file: {error}");
                                post_to_ui(move || {
                                    if let Some(ui) = weak.upgrade() {
                                        ui.set_status_text(message.into());
                                    }
                                });
                                return;
                            }
                        },
                        Ok(None) => {
                            post_to_ui(move || {
                                if let Some(ui) = weak.upgrade() {
                                    ui.set_status_text("File selection cancelled.".into());
                                }
                            });
                            return;
                        }
                        Err(error) => {
                            let message = format!("Could not open file picker: {error}");
                            post_to_ui(move || {
                                if let Some(ui) = weak.upgrade() {
                                    ui.set_status_text(message.into());
                                }
                            });
                            return;
                        }
                    };

                    let size = match local_file.size() {
                        Some(size) => size,
                        None => match std::fs::metadata(local_file.path()) {
                            Ok(metadata) => metadata.len(),
                            Err(error) => {
                                let message =
                                    format!("Could not read the selected file size: {error}");
                                post_to_ui(move || {
                                    if let Some(ui) = weak.upgrade() {
                                        ui.set_status_text(message.into());
                                    }
                                });
                                return;
                            }
                        },
                    };
                    if size > 10 * 1024 * 1024 {
                        post_to_ui(move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_status_text("File size must be 10 MB or less.".into());
                            }
                        });
                        return;
                    }

                    let file_name = local_file
                        .display_name()
                        .map(str::to_owned)
                        .or_else(|| {
                            local_file
                                .path()
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                        })
                        .unwrap_or_else(|| "Selected file".to_string());
                    let preview_path = local_file.path().to_path_buf();

                    post_to_ui(move || {
                        if let Some(ui) = weak.upgrade() {
                            let preview = slint::Image::load_from_path(&preview_path).ok();
                            ui.set_is_image(preview.is_some());
                            if let Some(preview) = preview {
                                ui.set_preview_image(preview);
                            }
                            *selected_file.lock().expect("Selected file lock poisoned") =
                                Some(local_file);
                            ui.set_file_name(file_name.into());
                            ui.set_has_file(true);
                            ui.set_can_upload(true);
                            ui.set_upload_complete(false);
                            ui.set_share_link("".into());
                            ui.set_progress(0);
                            ui.set_status_text("File selected.".into());
                        }
                    });
                });

            if let Err(error) = picker_result {
                ui.set_status_text(format!("Could not open file picker: {error}").into());
            }
        }
    });

    let weak = app.as_weak();
    let selected_file_for_remove = std::sync::Arc::clone(&selected_file);

    app.on_remove_file_requested(move || {
        if let Some(ui) = weak.upgrade() {
            *selected_file_for_remove
                .lock()
                .expect("Selected file lock poisoned") = None;
            ui.set_has_file(false);
            ui.set_file_name("No file selected".into());
            ui.set_is_image(false);
            ui.set_preview_image(slint::Image::default());
            ui.set_upload_complete(false);
            ui.set_share_link("".into());
            ui.set_progress(0);
            ui.set_status_text("File selection cleared.".into());
        }
    });

    let weak = app.as_weak();
    let selected_file_for_upload = std::sync::Arc::clone(&selected_file);

    app.on_upload_requested(move || {
        if let Some(ui) = weak.upgrade() {
            let file = selected_file_for_upload
                .lock()
                .expect("Selected file lock poisoned")
                .clone();
            let Some(file) = file else {
                ui.set_status_text("Select a file before uploading.".into());
                return;
            };

            ui.set_is_uploading(true);
            ui.set_progress(10);
            ui.set_status_text("Starting upload...".into());
            ui.set_upload_complete(false);
            ui.set_share_link("".into());

            let weak = weak.clone();
            let result = std::thread::Builder::new()
                .name("rustdrop-upload".to_string())
                .spawn(move || {
                    let result = upload_selected_file(file);
                    post_to_ui(move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_is_uploading(false);
                            match result {
                                Ok(link) => {
                                    ui.set_progress(100);
                                    ui.set_share_link(link.into());
                                    match services::qr::image_for_url(&ui.get_share_link()) {
                                        Ok(image) => {
                                            ui.set_qr_image(image);
                                            ui.set_status_text(
                                                "File uploaded successfully.".into(),
                                            );
                                        }
                                        Err(error) => {
                                            eprintln!("Could not generate QR code: {error}");
                                            ui.set_status_text(
                                                "File uploaded, but could not generate a QR code."
                                                    .into(),
                                            );
                                        }
                                    }
                                    ui.set_upload_complete(true);
                                }
                                Err(error) => {
                                    ui.set_progress(0);
                                    ui.set_status_text(format!("Upload failed: {error}").into());
                                }
                            }
                        }
                    });
                });

            if let Err(error) = result {
                ui.set_is_uploading(false);
                ui.set_progress(0);
                ui.set_status_text(format!("Could not start upload: {error}").into());
            }
        }
    });

    let weak = app.as_weak();

    app.on_copy_link_requested(move || {
        if let Some(ui) = weak.upgrade() {
            match clipboard::set_text(&ui.get_share_link()) {
                Ok(()) => ui.set_status_text("Link copied to clipboard.".into()),
                Err(error) => {
                    eprintln!("Could not copy share link: {error}");
                    ui.set_status_text(format!("Could not copy link: {error}").into());
                }
            }
        }
    });

    let weak = app.as_weak();

    app.on_download_qr_requested(move || {
        if let Some(ui) = weak.upgrade() {
            let png = match services::qr::png_for_url(&ui.get_share_link()) {
                Ok(png) => png,
                Err(error) => {
                    eprintln!("Could not encode QR code as PNG: {error}");
                    ui.set_status_text(format!("Could not prepare QR download: {error}").into());
                    return;
                }
            };

            let weak = weak.clone();
            let result = robius_file_picker::FileDialog::new()
                .set_title("Save QR code")
                .set_file_name("rustdrop-qr.png")
                .set_mime_type("image/png")
                .save_data(png, move |result| {
                    post_to_ui(move || {
                        if let Some(ui) = weak.upgrade() {
                            match result {
                                Ok(Some(_)) => ui.set_status_text("QR code saved.".into()),
                                Ok(None) => ui.set_status_text("QR code save cancelled.".into()),
                                Err(error) => {
                                    eprintln!("Could not save QR code: {error}");
                                    ui.set_status_text(
                                        format!("Could not save QR code: {error}").into(),
                                    );
                                }
                            }
                        }
                    });
                });

            if let Err(error) = result {
                eprintln!("Could not open QR save dialog: {error}");
                ui.set_status_text(format!("Could not save QR code: {error}").into());
            }
        }
    });

    app.run().expect("RustDrop UI failed");
}

fn upload_selected_file(file: robius_file_picker::LocalFile) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;

    runtime.block_on(async move {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let port = loop {
            if let Some(port) = server::port() {
                break port;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("File sharing server did not start.".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };

        services::upload::send_selected_file(file, port)
            .await
            .map_err(|error| error.to_string())
    })
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).expect("Android initialization failed");

    run();
}
