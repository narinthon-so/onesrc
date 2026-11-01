//! Masking for anything that may be a secret. Applied to every output.

pub const MASK: &str = "***";

const TOKEN_PREFIXES: &[&str] = &[
    "sk-",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "eyJ",
];
const SECRET_KEY_WORDS: &[&str] = &[
    "token", "secret", "password", "passwd", "apikey", "api_key", "key", "auth",
];

/// Mask a command-line argument or URL if it carries something token-like.
pub fn scrub(s: &str) -> String {
    // 1. Standalone tokens (e.g. ghp_..., or base64 token with '=' padding)
    if looks_like_token(s) {
        return MASK.to_string();
    }

    // 2. Key=value assignments where the key is a plausible flag or name
    if let Some((k, v)) = s.split_once('=') {
        if is_plausible_key(k) && !v.is_empty() {
            if is_secret_key(k) || looks_like_token(v) {
                return format!("{k}={MASK}");
            }
            return format!("{k}={}", scrub(v));
        }
    }

    // 3. HTTP/HTTPS URLs (case-insensitive scheme)
    if is_http_url(s) {
        return scrub_url(s);
    }

    s.to_string()
}

/// Scrub a list of command-line arguments, taking into account two-arg forms
/// like `["--token", "secret"]` where the value follows a secret-looking flag.
pub fn scrub_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut mask_next = false;
    for a in args {
        if mask_next {
            out.push(MASK.to_string());
            mask_next = false;
            continue;
        }
        if a.starts_with('-') && !a.contains('=') && is_secret_key(a) {
            mask_next = true;
            out.push(a.clone());
        } else {
            out.push(scrub(a));
        }
    }
    out
}

fn is_http_url(s: &str) -> bool {
    if let Some(prefix) = s.get(..7) {
        if prefix.eq_ignore_ascii_case("http://") {
            return true;
        }
    }
    if let Some(prefix) = s.get(..8) {
        if prefix.eq_ignore_ascii_case("https://") {
            return true;
        }
    }
    false
}

fn scrub_url(s: &str) -> String {
    let (base, query_opt) = match s.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (s, None),
    };
    let Some((scheme, rest)) = base.split_once("://") else {
        return s.to_string();
    };
    let (authority, path_opt) = match rest.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (rest, None),
    };

    // Scrub password/token in userinfo: https://user:password@host/ or https://token:pw@host/
    let scrubbed_auth = if let Some((userinfo, host)) = authority.split_once('@') {
        let scrubbed_userinfo = if let Some((user, _pw)) = userinfo.split_once(':') {
            if looks_like_token(user) || is_secret_key(user) {
                MASK.to_string()
            } else {
                format!("{user}:{MASK}")
            }
        } else if looks_like_token(userinfo) || is_secret_key(userinfo) {
            MASK.to_string()
        } else {
            userinfo.to_string()
        };
        format!("{scrubbed_userinfo}@{host}")
    } else {
        authority.to_string()
    };

    // Scrub token-shaped URL path segments: /mcp/ghp_...
    let scrubbed_path = path_opt
        .map(|p| {
            let segments: Vec<String> = p
                .split('/')
                .map(|seg| {
                    if looks_like_token(seg) {
                        MASK.to_string()
                    } else if let Some((k, v)) = seg.split_once('=') {
                        if is_secret_key(k) || looks_like_token(v) {
                            format!("{k}={MASK}")
                        } else {
                            seg.to_string()
                        }
                    } else {
                        seg.to_string()
                    }
                })
                .collect();
            format!("/{}", segments.join("/"))
        })
        .unwrap_or_default();

    let mut out = format!("{scheme}://{scrubbed_auth}{scrubbed_path}");
    if let Some(query) = query_opt {
        let q: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) if is_secret_key(k) || looks_like_token(v) => {
                    format!("{k}={MASK}")
                }
                _ => pair.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&q.join("&"));
    }
    out
}

fn is_secret_key(k: &str) -> bool {
    let k = k.trim_start_matches('-').to_ascii_lowercase();
    SECRET_KEY_WORDS.iter().any(|w| k.contains(w))
}

fn is_plausible_key(k: &str) -> bool {
    if k.is_empty() || k.contains('/') {
        return false;
    }
    let flag_body = k.trim_start_matches('-');
    if flag_body.is_empty() {
        return false;
    }
    // CLI flag: --token, -t, --api-key
    if k.starts_with('-') {
        return flag_body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    }
    // Known secret key word: token, secret, password, key, auth, API_KEY, etc.
    if is_secret_key(k) {
        return true;
    }
    // Environment variable: uppercase letters, digits, underscores (e.g. SERVICE_URL, PORT)
    if k.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return true;
    }
    // Config key: lowercase letters, digits, underscores, dashes (e.g. url, base_dir, my-flag)
    if k.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return true;
    }
    false
}

fn looks_like_token(s: &str) -> bool {
    if TOKEN_PREFIXES.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    // Cannot start with a flag dash
    if s.starts_with('-') {
        return false;
    }
    // Long run of base64/hex-ish characters with both letters and digits.
    // In base64, '=' is strictly padding at the end.
    let trimmed = s.trim_end_matches('=');
    if trimmed.contains('=') {
        return false;
    }
    s.len() >= 24
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_+/.".contains(c))
        && trimmed.chars().any(|c| c.is_ascii_digit())
        && trimmed.chars().any(|c| c.is_ascii_alphabetic())
        && !trimmed.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        assert_eq!(scrub("--token=abc"), "--token=***");
        assert_eq!(scrub("ghp_abcdef"), "***");
        assert_eq!(
            scrub("https://x.dev/mcp?key=abc&mode=1"),
            "https://x.dev/mcp?key=***&mode=1"
        );
        assert_eq!(scrub("Zx9Qw8Er7Ty6Ui5Op4As3Df2"), "***");
    }

    #[test]
    fn secret_key_with_url_value_is_masked() {
        assert_eq!(scrub("--token=https://example.com/hunter2"), "--token=***");
        assert_eq!(
            scrub("API_KEY=https://api.example.com/secret"),
            "API_KEY=***"
        );
    }

    #[test]
    fn token_shaped_url_username_is_masked() {
        assert_eq!(
            scrub("https://ghp_secrettoken123456789012:pw@example.com/mcp"),
            "https://***@example.com/mcp"
        );
        assert_eq!(
            scrub("https://ghp_secrettoken123456789012@example.com/mcp"),
            "https://***@example.com/mcp"
        );
        assert_eq!(
            scrub("https://user:hunter2@example.com/mcp"),
            "https://user:***@example.com/mcp"
        );
    }

    #[test]
    fn file_url_stays_intact() {
        let url = "file:///tmp/ghp_project/index.js";
        assert_eq!(scrub(url), url);
    }

    #[test]
    fn masks_url_userinfo() {
        assert_eq!(
            scrub("https://user:hunter2@example.com/mcp"),
            "https://user:***@example.com/mcp"
        );
    }

    #[test]
    fn url_path_token_is_masked() {
        assert_eq!(
            scrub("https://example.com/mcp/ghp_PLANTED0token0value0456"),
            "https://example.com/mcp/***"
        );
        assert_eq!(
            scrub("https://example.com/mcp/ghp_PLANTED0path0token0888"),
            "https://example.com/mcp/***"
        );
    }

    #[test]
    fn filesystem_path_with_long_random_segment_stays_intact() {
        let path = "/private/var/folders/n6/x9y8z7w6v5u4t3s2r1q0p9o8n7m6/T/tools/x/index.js";
        assert_eq!(scrub(path), path);
    }

    #[test]
    fn masks_two_arg_form() {
        let args = vec![
            "--token".into(),
            "hunter2".into(),
            "--api-key".into(),
            "x".into(),
        ];
        assert_eq!(
            scrub_args(&args),
            vec!["--token", "***", "--api-key", "***"]
        );
    }

    #[test]
    fn keeps_ordinary() {
        assert_eq!(
            scrub("/opt/tools/example-mcp/dist/index.js"),
            "/opt/tools/example-mcp/dist/index.js"
        );
        assert_eq!(
            scrub("@modelcontextprotocol/server-filesystem"),
            "@modelcontextprotocol/server-filesystem"
        );
        assert_eq!(scrub("-y"), "-y");
        assert_eq!(scrub("https://example.com/mcp"), "https://example.com/mcp");
    }

    #[test]
    fn padded_base64_token_is_masked() {
        assert_eq!(scrub("Zx9Qw8Er7Ty6Ui5Op4As3Df2=="), "***");
        assert_eq!(scrub("Zx9Qw8Er7Ty6Ui5Op4As3Df2="), "***");
        assert_eq!(scrub("Zx9Qw8Er7Ty6Ui5Op4As3Df2"), "***");
    }

    #[test]
    fn uppercase_and_mixed_case_scheme_masked() {
        assert_eq!(
            scrub("HTTPS://user:hunter2@example.com/mcp?key=abc"),
            "HTTPS://user:***@example.com/mcp?key=***"
        );
        assert_eq!(
            scrub("Http://user:hunter2@example.com/mcp?key=abc"),
            "Http://user:***@example.com/mcp?key=***"
        );
        assert_eq!(
            scrub("hTTpS://user:hunter2@example.com/mcp"),
            "hTTpS://user:***@example.com/mcp"
        );
        assert_eq!(
            scrub("HTTP://example.com/mcp?token=secret"),
            "HTTP://example.com/mcp?token=***"
        );
    }

    #[test]
    fn regression_table_of_earlier_cases() {
        // userinfo password
        assert_eq!(
            scrub("https://user:hunter2@example.com/mcp"),
            "https://user:***@example.com/mcp"
        );
        // token username
        assert_eq!(
            scrub("https://ghp_secrettoken123456789012:pw@example.com/mcp"),
            "https://***@example.com/mcp"
        );
        // --token value two-arg
        assert_eq!(
            scrub_args(&["--token".into(), "hunter2".into()]),
            vec!["--token", "***"]
        );
        // --token=https://...
        assert_eq!(scrub("--token=https://example.com/hunter2"), "--token=***");
        // URL path token
        assert_eq!(
            scrub("https://example.com/mcp/ghp_PLANTED0path0token0888"),
            "https://example.com/mcp/***"
        );
        // file:// path intact
        assert_eq!(
            scrub("file:///tmp/ghp_project/index.js"),
            "file:///tmp/ghp_project/index.js"
        );
        // long random filesystem segment intact
        assert_eq!(
            scrub("/private/var/folders/n6/x9y8z7w6v5u4t3s2r1q0p9o8n7m6/T/tools/x/index.js"),
            "/private/var/folders/n6/x9y8z7w6v5u4t3s2r1q0p9o8n7m6/T/tools/x/index.js"
        );
    }
}
