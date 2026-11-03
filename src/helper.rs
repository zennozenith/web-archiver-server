use crate::error::AppError;

pub(crate) fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

pub(crate) fn parse_bool(s: &str) -> Result<bool, AppError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::invalid(
            "invalid_save_assets",
            "save-assets must be true or false",
        )),
    }
}
