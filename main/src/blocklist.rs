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

/// Redact common secret patterns from text before it is sent to a provider.
pub fn redact_secrets(input: &str) -> String {
    let mut out = redact_pem(input);
    out = redact_assignments(&out);
    redact_token_prefixes(&out)
}

fn redact_pem(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("-----BEGIN ") {
        out.push_str(&rest[..start]);
        out.push_str("[REDACTED_PEM]");
        if let Some(end) = rest[start..].find("-----END ") {
            let after = &rest[start + end..];
            if let Some(line_end) = after.find('\n') {
                rest = &after[line_end + 1..];
            } else if let Some(dash_end) = after.find("-----") {
                // skip through trailing dashes
                let tail = &after[dash_end + 5..];
                rest = tail.trim_start_matches('-');
                if let Some(nl) = rest.find('\n') {
                    rest = &rest[nl + 1..];
                } else {
                    rest = "";
                }
            } else {
                rest = "";
            }
        } else {
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn redact_assignments(input: &str) -> String {
    let keys = [
        "api_key",
        "apikey",
        "api-key",
        "secret",
        "token",
        "password",
        "passwd",
        "authorization",
    ];
    let mut out = String::new();
    for line in input.lines() {
        let lower = line.to_ascii_lowercase();
        let mut redacted = false;
        for key in keys {
            if let Some(pos) = lower.find(key) {
                let after = &line[pos + key.len()..];
                if after.trim_start().starts_with('=') || after.trim_start().starts_with(':') {
                    if let Some(sep) = after.find(['=', ':']) {
                        let (left, _) = line.split_at(pos + key.len() + sep + 1);
                        out.push_str(left);
                        out.push_str(" [REDACTED]");
                        redacted = true;
                        break;
                    }
                }
            }
        }
        if !redacted {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !input.ends_with('\n') {
        out.pop();
    }
    out
}

fn redact_token_prefixes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == 's' || ch == 'g' || ch == 'x' {
            let mut token = String::new();
            token.push(ch);
            while let Some(next) = chars.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '-' || next == '_' {
                    token.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if token.starts_with("sk-") && token.len() >= 19
                || token.starts_with("ghp_") && token.len() >= 24
                || token.starts_with("xox") && token.len() >= 15
            {
                out.push_str("[REDACTED_TOKEN]");
            } else {
                out.push_str(&token);
            }
        } else {
            out.push(ch);
        }
    }
    out
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

    #[test]
    fn redacts_api_key_assignment_and_token_prefix() {
        let raw = "api_key=sk-abcdefghijklmnopqrstuvwxyz\nOPENAI_API_KEY: sk-abcdefghijklmnopqrstuvwxyz\n";
        let redacted = redact_secrets(raw);
        assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(redacted.contains("[REDACTED]") || redacted.contains("[REDACTED_TOKEN]"));
    }
}
