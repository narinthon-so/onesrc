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
const SECRET_KEY_COMPONENTS: &[&str] = &[
    "key",
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "auth",
    "apikey",
    "credential",
    "credentials",
    "bearer",
];

const SECRET_KEY_COMPOUNDS: &[&str] = &[
    "apikey",
    "accesskey",
    "secretkey",
    "privatekey",
    "authtoken",
    "accesstoken",
    "clientsecret",
];

/// Mask a command-line argument or URL if it carries something token-like.
pub fn scrub(s: &str) -> String {
    // 1. Header-style strings: Authorization: Bearer ..., x-api-key: value
    if let Some(res) = scrub_header(s) {
        return res;
    }

    // 2. Standalone tokens (e.g. ghp_..., or base64 token with '=' padding)
    if looks_like_token(s) {
        return MASK.to_string();
    }

    // 3. Key=value assignments: anything before the first '=', no gating on key shape.
    // A string with no '=' before any '/' or ':' is treated as a whole value.
    let res = if let Some((k, v)) = s.split_once('=') {
        if !k.contains('/') && !k.contains(':') && !v.is_empty() {
            if is_secret_key(k) {
                format!("{k}={MASK}")
            } else {
                format!("{k}={}", scrub(v))
            }
        } else if is_http_url(s) {
            scrub_url(s)
        } else {
            s.to_string()
        }
    } else if is_http_url(s) {
        scrub_url(s)
    } else {
        s.to_string()
    };

    // 4. Backstop: scan for token substrings
    backstop_mask(&res)
}

fn scrub_header(s: &str) -> Option<String> {
    if s.contains("://") {
        return None;
    }
    let (name, val) = s.split_once(':')?;
    let name_trimmed = name.trim();
    if name_trimmed.is_empty() || name_trimmed.contains('/') || name_trimmed.contains('=') {
        return None;
    }
    if !name_trimmed.eq_ignore_ascii_case("authorization") && !is_secret_key(name_trimmed) {
        return None;
    }
    let val_trimmed = val.trim_start();
    if val_trimmed.starts_with("Bearer ") || val_trimmed.starts_with("bearer ") {
        Some(format!("{name}: Bearer {MASK}"))
    } else {
        Some(format!("{name}: {MASK}"))
    }
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
    let (without_frag, frag_opt) = match s.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (s, None),
    };
    let (base, query_opt) = match without_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (without_frag, None),
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

    // Scrub URL path segments recursively: /mcp/ghp_... or /a=b=ghp_...
    let scrubbed_path = path_opt
        .map(|p| {
            let segments: Vec<String> = p.split('/').map(scrub).collect();
            format!("/{}", segments.join("/"))
        })
        .unwrap_or_default();

    let mut out = format!("{scheme}://{scrubbed_auth}{scrubbed_path}");
    if let Some(query) = query_opt {
        let q: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((k, _)) if is_secret_key(k) => {
                    format!("{k}={MASK}")
                }
                Some((k, v)) => {
                    format!("{k}={}", scrub(v))
                }
                None => scrub(pair),
            })
            .collect();
        out.push('?');
        out.push_str(&q.join("&"));
    }
    if let Some(frag) = frag_opt {
        out.push('#');
        if frag.contains('&') || frag.contains('=') {
            let pairs: Vec<String> = frag
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((k, _)) if is_secret_key(k) => {
                        format!("{k}={MASK}")
                    }
                    Some((k, v)) => {
                        format!("{k}={}", scrub(v))
                    }
                    None => scrub(pair),
                })
                .collect();
            out.push_str(&pairs.join("&"));
        } else {
            out.push_str(&scrub(frag));
        }
    }
    out
}

fn split_key_components(k: &str) -> Vec<String> {
    let k = k.trim_start_matches('-');
    let mut components = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = k.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];
        if c == '-' || c == '_' || c == '.' {
            if !current.is_empty() {
                components.push(current.to_ascii_lowercase());
                current = String::new();
            }
            continue;
        }

        // camelCase boundary: lower/digit followed by uppercase (e.g. clientSecret)
        if !current.is_empty()
            && c.is_ascii_uppercase()
            && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit())
        {
            components.push(current.to_ascii_lowercase());
            current = String::new();
        }
        // camelCase boundary: acronym followed by Capitalized word (e.g. APIKey)
        else if !current.is_empty()
            && c.is_ascii_lowercase()
            && current.len() > 1
            && chars[i - 1].is_ascii_uppercase()
        {
            let prev = current.pop().unwrap();
            components.push(current.to_ascii_lowercase());
            current = String::new();
            current.push(prev);
        }

        current.push(c);
    }

    if !current.is_empty() {
        components.push(current.to_ascii_lowercase());
    }

    components
}

fn is_secret_key(k: &str) -> bool {
    let k_trimmed = k.trim_start_matches('-');
    let lower = k_trimmed.to_ascii_lowercase();

    if SECRET_KEY_COMPOUNDS.contains(&lower.as_str()) {
        return true;
    }

    let components = split_key_components(k);
    components
        .iter()
        .any(|c| SECRET_KEY_COMPONENTS.contains(&c.as_str()))
}

fn is_filesystem_path(s: &str) -> bool {
    (s.starts_with('/')
        || s.starts_with("file://")
        || s.starts_with("{home}")
        || s.starts_with("{source}")
        || s.starts_with('~'))
        && !is_http_url(s)
}

fn backstop_mask(s: &str) -> String {
    // Phase 1: replace known-prefix tokens anywhere in the string
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < s.len() {
        let mut matched = false;
        for &prefix in TOKEN_PREFIXES {
            if s[i..].starts_with(prefix) {
                let mut end = i + prefix.len();
                while end < s.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'_'
                        || bytes[end] == b'-')
                {
                    end += 1;
                }
                if end < s.len()
                    && bytes[end] == b'='
                    && (end + 1 == s.len()
                        || bytes[end + 1] == b'='
                        || !bytes[end + 1].is_ascii_alphanumeric())
                {
                    end += 1;
                    if end < s.len()
                        && bytes[end] == b'='
                        && (end + 1 == s.len() || !bytes[end + 1].is_ascii_alphanumeric())
                    {
                        end += 1;
                    }
                }
                let token = &s[i..end];
                if token.len() >= 16 {
                    out.push_str(MASK);
                    i = end;
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }

        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    // Phase 2: if not a filesystem path, check for long-mixed-class tokens
    if is_filesystem_path(&out) {
        return out;
    }

    let mut final_out = String::with_capacity(out.len());
    let out_bytes = out.as_bytes();
    let mut j = 0;

    while j < out.len() {
        if out_bytes[j].is_ascii_alphanumeric()
            || out_bytes[j] == b'_'
            || out_bytes[j] == b'-'
            || out_bytes[j] == b'+'
            || out_bytes[j] == b'.'
        {
            let start = j;
            let mut end = j;
            while end < out.len()
                && (out_bytes[end].is_ascii_alphanumeric()
                    || out_bytes[end] == b'_'
                    || out_bytes[end] == b'-'
                    || out_bytes[end] == b'+'
                    || out_bytes[end] == b'.')
            {
                end += 1;
            }
            if end < out.len()
                && out_bytes[end] == b'='
                && (end + 1 == out.len()
                    || out_bytes[end + 1] == b'='
                    || !out_bytes[end + 1].is_ascii_alphanumeric())
            {
                end += 1;
                if end < out.len()
                    && out_bytes[end] == b'='
                    && (end + 1 == out.len() || !out_bytes[end + 1].is_ascii_alphanumeric())
                {
                    end += 1;
                }
            }
            let run = &out[start..end];
            let trimmed = run.trim_end_matches('=');
            if run.len() >= 24
                && !trimmed.contains('=')
                && trimmed.chars().any(|c| c.is_ascii_digit())
                && trimmed.chars().any(|c| c.is_ascii_alphabetic())
                && !run.starts_with('-')
            {
                final_out.push_str(MASK);
                j = end;
                continue;
            }
        }

        let ch = out[j..].chars().next().unwrap();
        final_out.push(ch);
        j += ch.len_utf8();
    }

    final_out
}

fn looks_like_token(s: &str) -> bool {
    // Cannot start with a flag dash
    if s.starts_with('-') {
        return false;
    }
    // In base64/tokens, '=' is strictly padding at the end; interior '=' means assignment.
    let trimmed = s.trim_end_matches('=');
    if trimmed.contains('=') {
        return false;
    }
    if TOKEN_PREFIXES.iter().any(|p| s.starts_with(p)) {
        return true;
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

    #[test]
    fn table_driven_redaction_cases() {
        let cases: &[(&str, &str)] = &[
            // userinfo password
            (
                "https://user:hunter2@example.com/mcp",
                "https://user:***@example.com/mcp",
            ),
            // token username
            (
                "https://ghp_secrettoken123456789012:pw@example.com/mcp",
                "https://***@example.com/mcp",
            ),
            (
                "https://ghp_secrettoken123456789012@example.com/mcp",
                "https://***@example.com/mcp",
            ),
            // --token=https://...
            ("--token=https://example.com/hunter2", "--token=***"),
            // URL path token
            (
                "https://example.com/mcp/ghp_PLANTED0path0token0888",
                "https://example.com/mcp/***",
            ),
            // file:// path intact
            (
                "file:///tmp/ghp_project/index.js",
                "file:///tmp/ghp_project/index.js",
            ),
            // long random filesystem segment intact
            (
                "/private/var/folders/n6/x9y8z7w6v5u4t3s2r1q0p9o8n7m6/T/tools/x/index.js",
                "/private/var/folders/n6/x9y8z7w6v5u4t3s2r1q0p9o8n7m6/T/tools/x/index.js",
            ),
            // padded standalone token
            ("Zx9Qw8Er7Ty6Ui5Op4As3Df2==", "***"),
            ("Zx9Qw8Er7Ty6Ui5Op4As3Df2=", "***"),
            ("Zx9Qw8Er7Ty6Ui5Op4As3Df2", "***"),
            // uppercase/mixed-case HTTPS
            (
                "HTTPS://user:hunter2@example.com/mcp?key=abc",
                "HTTPS://user:***@example.com/mcp?key=***",
            ),
            (
                "Http://user:hunter2@example.com/mcp?key=abc",
                "Http://user:***@example.com/mcp?key=***",
            ),
            (
                "hTTpS://user:hunter2@example.com/mcp",
                "hTTpS://user:***@example.com/mcp",
            ),
            // clientId=<token>
            ("clientId=Zx9Qw8Er7Ty6Ui5Op4As3Df2", "clientId=***"),
            // --client.id=<token>
            ("--client.id=Zx9Qw8Er7Ty6Ui5Op4As3Df2", "--client.id=***"),
            // empty key with token value
            ("=ghp_PLANTEDsecret", "=***"),
            // URL path-segment assignment with token
            (
                "https://example.com/a=b=ghp_PLANTEDsecret",
                "https://example.com/a=b=***",
            ),
            // URL query assignment with token
            (
                "https://example.com/?mode=1&clientId=x=ghp_PLANTEDsecret&n=2",
                "https://example.com/?mode=1&clientId=x=***&n=2",
            ),
            // URL fragment tokens
            (
                "https://example.com/#ghp_PLANTEDsecret",
                "https://example.com/#***",
            ),
            (
                "https://example.com/?mode=1#ghp_PLANTEDsecret",
                "https://example.com/?mode=1#***",
            ),
            // secret-key word components vs non-secret substrings
            ("--monkey=banana", "--monkey=banana"),
            ("clientSecret=secret123", "clientSecret=***"),
            ("API_KEY=secret123", "API_KEY=***"),
            ("x-api-key=secret123", "x-api-key=***"),
            // A. Backstop token substrings
            (
                "https://e.test/#/ghp_PLANTED0secret012345678901234567890",
                "https://e.test/#/***",
            ),
            (
                "https://e.test/#/route?token=ghp_PLANTED0secret012345678901234567890",
                "https://e.test/#/route?token=***",
            ),
            (
                "https://e.test/?v=x%3Dghp_PLANTED0secret012345678901234567890",
                "https://e.test/?v=x%3D***",
            ),
            ("/ghp_PLANTED0secret012345678901234567890=x", "/***=x"),
            ("?ghp_PLANTED0secret012345678901234567890=x", "?***=x"),
            // B. Header-style strings
            (
                "Authorization: Bearer ghp_PLANTED0secret012345678901234567890",
                "Authorization: Bearer ***",
            ),
            (
                "Authorization: Bearer shortsecret",
                "Authorization: Bearer ***",
            ),
            ("x-api-key: value", "x-api-key: ***"),
            ("X-Auth-Token: value", "X-Auth-Token: ***"),
            // C. Over-masking: drop generic 'ends with key' rule
            ("--hotkey=x", "--hotkey=x"),
            // D. Fragment pairs
            (
                "https://example.com/#access_token=abc&state=xyz",
                "https://example.com/#access_token=***&state=xyz",
            ),
            // Passing probes
            ("--keyboard-layout us", "--keyboard-layout us"),
            ("--tokenizer bpe", "--tokenizer bpe"),
            ("AUTH_HEADER=Bearer abc123secret", "AUTH_HEADER=***"),
            // ordinary flags unchanged
            ("-y", "-y"),
            ("--verbose", "--verbose"),
            // ordinary paths unchanged
            (
                "/opt/tools/example-mcp/dist/index.js",
                "/opt/tools/example-mcp/dist/index.js",
            ),
            (
                "@modelcontextprotocol/server-filesystem",
                "@modelcontextprotocol/server-filesystem",
            ),
            ("https://example.com/mcp", "https://example.com/mcp"),
        ];

        for &(input, expected) in cases {
            assert_eq!(scrub(input), expected, "failed on scrub({input:?})");
        }

        // --token value two-arg
        assert_eq!(
            scrub_args(&["--token".into(), "hunter2".into()]),
            vec!["--token", "***"]
        );
        assert_eq!(
            scrub_args(&["--api-key".into(), "x".into()]),
            vec!["--api-key", "***"]
        );
        assert_eq!(
            scrub_args(&["--author".into(), "Jane".into(), "--verbose".into()]),
            vec!["--author", "Jane", "--verbose"]
        );
        assert_eq!(
            scrub_args(&["--api_key".into(), "value".into()]),
            vec!["--api_key", "***"]
        );
    }
}
