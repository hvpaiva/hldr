//! Painting a page with a theme from content. Colors are `#rrggbb` by the
//! time they get here (`hldr_core::types::Color`), so they can go into CSS
//! and SVG as they are.

use axum::http::{HeaderMap, header};
use hldr_core::Theme;

pub const COOKIE: &str = "theme";

pub fn color_scheme(theme: &Theme) -> &'static str {
    if theme.dark { "dark" } else { "light" }
}

pub fn root_css(theme: &Theme) -> String {
    let c = &theme.colors;
    format!(
        ":root{{color-scheme:{};--bg:{};--fg:{};--fg-dim:{};--fg-muted:{};--accent:{};--accent-dim:{}18;--accent2:{};--highlight:{};--special:{};--teal:{};--error:{};--border:{}}}",
        color_scheme(theme),
        c.bg,
        c.fg,
        c.fg_dim,
        c.fg_muted,
        c.accent,
        c.accent,
        c.accent2,
        c.highlight,
        c.special,
        c.teal,
        c.error,
        c.border
    )
}

/// The palette as `--t-*` custom properties. Scoped with a prefix so
/// carrying them on an element does not recolour that element; the
/// colorscheme preview copies them onto `:root`.
pub fn vars(theme: &Theme) -> String {
    let c = &theme.colors;
    format!(
        "--t-bg:{};--t-fg:{};--t-fg-dim:{};--t-fg-muted:{};--t-accent:{};--t-accent2:{};--t-highlight:{};--t-special:{};--t-teal:{};--t-error:{};--t-border:{};--t-scheme:{}",
        c.bg,
        c.fg,
        c.fg_dim,
        c.fg_muted,
        c.accent,
        c.accent2,
        c.highlight,
        c.special,
        c.teal,
        c.error,
        c.border,
        color_scheme(theme)
    )
}

pub fn favicon_svg(theme: &Theme) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" rx="8" fill="{bg}"/><text x="50" y="42" text-anchor="middle" font-family="monospace" font-size="38" font-weight="700" fill="{accent}">HL</text><text x="50" y="82" text-anchor="middle" font-family="monospace" font-size="38" font-weight="700" fill="{accent}">DR</text></svg>"#,
        bg = theme.colors.bg,
        accent = theme.colors.accent
    )
}

/// The slug is a validated content name, so it is safe in a cookie value.
pub fn cookie_header(theme: &Theme) -> String {
    format!(
        "{COOKIE}={}; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly; Secure",
        theme.slug
    )
}

/// The theme the visitor picked, if any; it may name a theme that no longer
/// exists.
pub fn cookie_slug(headers: &HeaderMap) -> Option<&str> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let part = part.trim();
        let (key, value) = part.split_once('=')?;
        (key == COOKIE).then_some(value)
    })
}

pub fn safe_return(headers: &HeaderMap) -> String {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return "/".into();
    };
    let Some(referer) = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
    else {
        return "/".into();
    };
    let Some(rest) = referer.split_once("://").map(|(_, rest)| rest) else {
        return "/".into();
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return "/".into();
    };
    if authority != host {
        return "/".into();
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let path = format!("/{path}");
    if path.starts_with("/theme/") {
        return "/".into();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use hldr_core::types::{Color, Palette};

    fn retro() -> Theme {
        let c = |hex: &str| Color::try_from(hex.to_owned()).unwrap();
        Theme {
            slug: "retro-82".to_owned(),
            title: "Retro 82".to_owned(),
            dark: true,
            order: None,
            hidden: false,
            colors: Palette {
                bg: c("#05182e"),
                fg: c("#f6dcac"),
                fg_dim: c("#a7c9c6"),
                fg_muted: c("#3f8f8a"),
                accent: c("#faa968"),
                accent2: c("#3f8f8a"),
                highlight: c("#e97b3c"),
                special: c("#3f8f8a"),
                teal: c("#8cbfb8"),
                error: c("#f85525"),
                border: c("#0a2540"),
            },
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn cookie_is_httponly_secure() {
        let header = cookie_header(&retro());
        assert!(header.starts_with("theme=retro-82;"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("Secure"));
        assert!(header.contains("SameSite=Lax"));
    }

    #[test]
    fn cookie_names_the_picked_theme() {
        let mut headers = HeaderMap::new();
        assert_eq!(cookie_slug(&headers), None);
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; theme=nord"),
        );
        assert_eq!(cookie_slug(&headers), Some("nord"));
    }
    #[test]
    fn return_path_stays_on_host_and_rejects_theme_loop() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("hvpaiva.dev"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://hvpaiva.dev/projects"),
        );
        assert_eq!(safe_return(&headers), "/projects");
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://evil.example/phish"),
        );
        assert_eq!(safe_return(&headers), "/");
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://hvpaiva.dev/theme/nord"),
        );
        assert_eq!(safe_return(&headers), "/");
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://hvpaiva.dev/theme"),
        );
        assert_eq!(safe_return(&headers), "/theme");
    }

    #[test]
    fn root_css_sets_tokens() {
        let css = root_css(&retro());
        assert!(css.contains("--bg:#05182e"));
        assert!(css.contains("--accent:#faa968"));
        assert!(css.contains("--accent-dim:#faa96818"));
        assert!(css.contains("color-scheme:dark"));
    }

    #[test]
    fn vars_are_prefixed_and_complete() {
        let vars = vars(&retro());
        assert!(vars.starts_with("--t-bg:#05182e;"));
        assert!(vars.contains("--t-accent:#faa968"));
        assert!(vars.ends_with("--t-scheme:dark"));
        assert_eq!(vars.matches("--t-").count(), 12);
        assert!(!vars.contains(";--bg"));
    }

    #[test]
    fn favicon_uses_theme_colors() {
        let svg = favicon_svg(&retro());
        assert!(svg.contains("fill=\"#05182e\""));
        assert!(svg.contains("fill=\"#faa968\""));
        assert!(svg.contains(">HL</text>"));
    }
}
