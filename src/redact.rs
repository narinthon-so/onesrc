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
    // 1. Process key=value assignments first (e.g. --token=https://example.com/hunter2)
    if let Some((k, v)) = s.split_once('=') {
        if !k.contains('/') && !v.is_empty() {
            if is_secret_key(k) || looks_like_token(v) {
                return format!("{k}={MASK}");
            }
            return format!("{k}={}", scrub(v));
        }
    }

    // 2. HTTP/HTTPS URLs: scrub userinfo, token path segments, and query string
    if is_http_url(s) {
        return scrub_url(s);
    }

    // 3. Standalone tokens
    if looks_like_token(s) {
        return MASK.to_string();
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
    s.starts_with("http://") || s.starts_with("https://")
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

fn looks_like_token(s: &str) -> bool {
    if TOKEN_PREFIXES.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    // long run of base64/hex-ish characters with both letters and digits
    s.len() >= 24
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_+/=.".contains(c))
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && !s.contains('/')
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
}
