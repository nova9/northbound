pub(crate) fn copy_text(value: &str, label: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| format!("Could not access the clipboard: {error}"))?;

    clipboard
        .set_text(value)
        .map_err(|error| format!("Could not copy {label}: {error}"))
}
