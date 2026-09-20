use axum::http::{HeaderMap, header};

pub fn strip_txt(value: &str) -> (&str, bool) {
    value
        .strip_suffix(".txt")
        .map_or((value, false), |stripped| (stripped, true))
}

pub fn wants_text(headers: &HeaderMap, force: bool) -> bool {
    if force {
        return true;
    }
    if let Some(accept) = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) {
        let html = accept.contains("text/html");
        let plain = accept.contains("text/plain");
        if html && !plain {
            return false;
        }
        if plain && !html {
            return true;
        }
    }
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(is_terminal_ua)
}

pub fn wants_color(headers: &HeaderMap, force_txt: bool) -> bool {
    if force_txt {
        return false;
    }
    !is_bot(headers)
}

fn is_terminal_ua(ua: &str) -> bool {
    let ua = ua.to_ascii_lowercase();
    ua.starts_with("curl/")
        || ua.starts_with("wget/")
        || ua.starts_with("httpie")
        || ua.starts_with("lynx")
        || ua.starts_with("w3m")
}

fn is_bot(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| {
            let ua = ua.to_ascii_lowercase();
            ua.contains("bot") || ua.contains("crawler") || ua.contains("spider")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(accept: Option<&str>, ua: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(accept) = accept {
            map.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
        }
        if let Some(ua) = ua {
            map.insert(header::USER_AGENT, HeaderValue::from_str(ua).unwrap());
        }
        map
    }

    #[test]
    fn curl_gets_text() {
        assert!(wants_text(&headers(Some("*/*"), Some("curl/8.5.0")), false));
    }

    #[test]
    fn browser_gets_html() {
        assert!(!wants_text(
            &headers(
                Some("text/html,application/xhtml+xml;q=0.9,*/*;q=0.8"),
                Some("Mozilla/5.0")
            ),
            false
        ));
    }

    #[test]
    fn explicit_plain_wins() {
        assert!(wants_text(
            &headers(Some("text/plain"), Some("Mozilla/5.0")),
            false
        ));
    }

    #[test]
    fn suffix_forces_text_without_color() {
        let headers = headers(Some("*/*"), Some("curl/8.5.0"));
        assert!(wants_text(&headers, true));
        assert!(!wants_color(&headers, true));
    }
}
