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
    // --flag=value or KEY=value
    if let Some((k, v)) = s.split_once('=') {
        if !k.contains('/') && !v.is_empty() && (is_secret_key(k) || looks_like_token(v)) {
            return format!("{k}={MASK}");
        }
    }
    // URL query string: mask values of secret-looking keys or token-like values
    if let Some((base, query)) = s.split_once('?') {
        if base.contains("://") {
            let q: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((k, v)) if is_secret_key(k) || looks_like_token(v) => {
                        format!("{k}={MASK}")
                    }
                    _ => pair.to_string(),
                })
                .collect();
            return format!("{base}?{}", q.join("&"));
        }
    }
    if looks_like_token(s) {
        return MASK.to_string();
    }
    s.to_string()
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
