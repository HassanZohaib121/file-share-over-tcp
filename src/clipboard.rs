#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) fn set_text(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(text).map_err(|error| error.to_string())
}

#[cfg(target_os = "android")]
pub(crate) fn set_text(text: &str) -> Result<(), String> {
    use jni::objects::JValue;

    robius_android_env::with_activity(|env, activity| {
        let service_name = env
            .new_string("clipboard")
            .map_err(|error| error.to_string())?;
        let manager = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(service_name.as_ref())],
            )
            .and_then(|value| value.l())
            .map_err(|error| error.to_string())?;
        let label = env
            .new_string("RustDrop link")
            .map_err(|error| error.to_string())?;
        let text = env.new_string(text).map_err(|error| error.to_string())?;
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[
                    JValue::Object(label.as_ref()),
                    JValue::Object(text.as_ref()),
                ],
            )
            .and_then(|value| value.l())
            .map_err(|error| error.to_string())?;
        env.call_method(
            manager,
            "setPrimaryClip",
            "(Landroid/content/ClipData;)V",
            &[JValue::Object(&clip)],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    })
    .map_err(|error| error.to_string())?
}

#[cfg(not(any(
    target_os = "android",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
)))]
pub(crate) fn set_text(_text: &str) -> Result<(), String> {
    Err("Clipboard is not supported on this platform.".to_string())
}
