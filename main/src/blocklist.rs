//! Shared file-name blocklist used by context selection, the optimizer, and the
//! tool-use loop to avoid leaking secrets or scanning irrelevant large files.
//!
//! Every module that decides which files to show to the model should call
//! [`is_secret_file`] and/or [`is_binary_or_lock_file`] so the deny-list stays
//! in one place.

use std::path::Path;

/// File names that are excluded from context, optimization, grep, and glob
/// results to avoid leaking credentials. Comparison is case-insensitive.
const SECRET_FILE_NAMES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    ".npmrc",
    ".pypirc",
    "secrets.yaml",
    "secrets.yml",
    "secrets.json",
    "credentials",
    "credentials.json",
    "serviceaccount.json",
    "api_keys.local.rs",
    "id_rsa",
    "id_ed25519",
];

/// Extensions that indicate binary or lock files which should be skipped during
/// automatic context selection.
const BINARY_EXTENSIONS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".pdf", ".zip", ".lock"];

/// Returns true when the file name matches a known secret/credential file.
/// Comparison is case-insensitive.
pub fn is_secret_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();
    SECRET_FILE_NAMES.iter().any(|s| lower == *s)
}

/// Returns true when the file has a binary or lock extension that should be
/// skipped during automatic context selection.
pub fn is_binary_or_lock_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();
    BINARY_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Convenience: true when the file should be skipped entirely (secret or binary).
pub fn should_skip(path: &Path) -> bool {
    is_secret_file(path) || is_binary_or_lock_file(path)
}

/// Returns the secret file name list for tools that need to pass `--exclude`
/// flags to external commands like grep.
pub fn secret_file_names() -> &'static [&'static str] {
    SECRET_FILE_NAMES
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_env_files() {
        assert!(is_secret_file(&PathBuf::from(".env")));
        assert!(is_secret_file(&PathBuf::from(".ENV")));
        assert!(is_secret_file(&PathBuf::from("secrets.yaml")));
        assert!(!is_secret_file(&PathBuf::from("main.rs")));
    }

    #[test]
    fn detects_binary_extensions() {
        assert!(is_binary_or_lock_file(&PathBuf::from("logo.png")));
        assert!(is_binary_or_lock_file(&PathBuf::from("Cargo.lock")));
        assert!(!is_binary_or_lock_file(&PathBuf::from("main.rs")));
    }

    #[test]
    fn should_skip_combines_both() {
        assert!(should_skip(&PathBuf::from(".env")));
        assert!(should_skip(&PathBuf::from("image.jpg")));
        assert!(!should_skip(&PathBuf::from("src/lib.rs")));
    }
}
