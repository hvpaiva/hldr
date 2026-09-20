use axum::http::{HeaderMap, header};

pub const COOKIE: &str = "theme";
pub const DEFAULT_SLUG: &str = "retro-82";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub slug: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub bg: &'static str,
    pub fg: &'static str,
    pub fg_dim: &'static str,
    pub fg_muted: &'static str,
    pub accent: &'static str,
    pub accent2: &'static str,
    pub highlight: &'static str,
    pub special: &'static str,
    pub teal: &'static str,
    pub error: &'static str,
    pub border: &'static str,
}

pub const THEMES: &[Theme] = &[
    Theme {
        slug: "all-hallows-eve",
        name: "All Hallows Eve",
        dark: true,
        bg: "#000000",
        fg: "#ffffff",
        fg_dim: "#ffffff",
        fg_muted: "#404040",
        accent: "#66cc33",
        accent2: "#3387cc",
        highlight: "#cc7833",
        special: "#9933cc",
        teal: "#cccc33",
        error: "#c83730",
        border: "#404040",
    },
    Theme {
        slug: "blueridge-dark",
        name: "Blueridge Dark",
        dark: true,
        bg: "#1c2128",
        fg: "#e8e6e3",
        fg_dim: "#ffffff",
        fg_muted: "#4a5568",
        accent: "#7ea67c",
        accent2: "#6b8cae",
        highlight: "#d4af37",
        special: "#9b8aa0",
        teal: "#7ec8c8",
        error: "#cd9575",
        border: "#4a5568",
    },
    Theme {
        slug: "catppuccin",
        name: "Catppuccin",
        dark: true,
        bg: "#1e1e2e",
        fg: "#cdd6f4",
        fg_dim: "#bac2de",
        fg_muted: "#6c7086",
        accent: "#89b4fa",
        accent2: "#89b4fa",
        highlight: "#f9e2af",
        special: "#f5c2e7",
        teal: "#94e2d5",
        error: "#f38ba8",
        border: "#313244",
    },
    Theme {
        slug: "catppuccin-dark",
        name: "Catppuccin Dark",
        dark: true,
        bg: "#0f0f0f",
        fg: "#cdd6f4",
        fg_dim: "#cdd6f4",
        fg_muted: "#7f849c",
        accent: "#a6e3a1",
        accent2: "#89b4fa",
        highlight: "#f9e2af",
        special: "#f5c2e7",
        teal: "#94e2d5",
        error: "#f38ba8",
        border: "#585b70",
    },
    Theme {
        slug: "catppuccin-latte",
        name: "Catppuccin Latte",
        dark: false,
        bg: "#eff1f5",
        fg: "#4c4f69",
        fg_dim: "#5c5f77",
        fg_muted: "#9ca0b0",
        accent: "#1e66f5",
        accent2: "#1e66f5",
        highlight: "#df8e1d",
        special: "#ea76cb",
        teal: "#179299",
        error: "#d20f39",
        border: "#dce0e8",
    },
    Theme {
        slug: "darcula",
        name: "Darcula",
        dark: true,
        bg: "#2b2b2b",
        fg: "#a9b7c6",
        fg_dim: "#ffffff",
        fg_muted: "#808080",
        accent: "#629755",
        accent2: "#6897bb",
        highlight: "#cc7832",
        special: "#9876aa",
        teal: "#6a8759",
        error: "#ff6b68",
        border: "#3c3f41",
    },
    Theme {
        slug: "ethereal",
        name: "Ethereal",
        dark: true,
        bg: "#060b1e",
        fg: "#ffcead",
        fg_dim: "#c9b8a6",
        fg_muted: "#6d7db6",
        accent: "#7d82d9",
        accent2: "#7d82d9",
        highlight: "#e9bb4f",
        special: "#c89dc1",
        teal: "#a3bfd1",
        error: "#ed5b5a",
        border: "#131a3a",
    },
    Theme {
        slug: "everforest",
        name: "Everforest",
        dark: true,
        bg: "#2d353b",
        fg: "#d3c6aa",
        fg_dim: "#9da9a0",
        fg_muted: "#4f585e",
        accent: "#7fbbb3",
        accent2: "#7fbbb3",
        highlight: "#dbbc7f",
        special: "#d699b6",
        teal: "#83c092",
        error: "#e67e80",
        border: "#343f44",
    },
    Theme {
        slug: "flexoki-dark",
        name: "Flexoki Dark",
        dark: true,
        bg: "#100f0f",
        fg: "#fffcf0",
        fg_dim: "#fffcf0",
        fg_muted: "#fffcf0",
        accent: "#66800b",
        accent2: "#205ea6",
        highlight: "#ad8301",
        special: "#a02f6f",
        teal: "#24837b",
        error: "#af3029",
        border: "#100f0f",
    },
    Theme {
        slug: "flexoki-light",
        name: "Flexoki Light",
        dark: false,
        bg: "#fffcf0",
        fg: "#100f0f",
        fg_dim: "#403e3c",
        fg_muted: "#878580",
        accent: "#205ea6",
        accent2: "#205ea6",
        highlight: "#d0a215",
        special: "#ce5d97",
        teal: "#3aa99f",
        error: "#d14d41",
        border: "#e6e4d9",
    },
    Theme {
        slug: "gruvbox",
        name: "Gruvbox",
        dark: true,
        bg: "#282828",
        fg: "#d4be98",
        fg_dim: "#bdae93",
        fg_muted: "#7c6f64",
        accent: "#7daea3",
        accent2: "#7daea3",
        highlight: "#d8a657",
        special: "#d3869b",
        teal: "#89b482",
        error: "#ea6962",
        border: "#3c3836",
    },
    Theme {
        slug: "hackerman",
        name: "Hackerman",
        dark: true,
        bg: "#0b0c16",
        fg: "#ddf7ff",
        fg_dim: "#b5c5db",
        fg_muted: "#6a6e95",
        accent: "#82fb9c",
        accent2: "#829dd4",
        highlight: "#50f7d4",
        special: "#86a7df",
        teal: "#7cf8f7",
        error: "#50f872",
        border: "#151828",
    },
    Theme {
        slug: "kanagawa",
        name: "Kanagawa",
        dark: true,
        bg: "#1f1f28",
        fg: "#dcd7ba",
        fg_dim: "#c8c093",
        fg_muted: "#727169",
        accent: "#dcd7ba",
        accent2: "#7e9cd8",
        highlight: "#c0a36e",
        special: "#957fb8",
        teal: "#6a9589",
        error: "#c34043",
        border: "#223249",
    },
    Theme {
        slug: "last-horizon",
        name: "Last Horizon",
        dark: true,
        bg: "#0c0b0c",
        fg: "#fafcfb",
        fg_dim: "#cfd3cd",
        fg_muted: "#584e51",
        accent: "#b59790",
        accent2: "#b59790",
        highlight: "#6b5e73",
        special: "#c4d8e2",
        teal: "#a5a0b6",
        error: "#c38b7b",
        border: "#0c0b0c",
    },
    Theme {
        slug: "lumon",
        name: "Lumon",
        dark: true,
        bg: "#16242d",
        fg: "#d6e2ee",
        fg_dim: "#d6e2ee",
        fg_muted: "#4d86b0",
        accent: "#8bc9eb",
        accent2: "#6fb8e3",
        highlight: "#6fa4c9",
        special: "#8bc9eb",
        teal: "#b4e4f6",
        error: "#4d86b0",
        border: "#1b2d40",
    },
    Theme {
        slug: "lupine",
        name: "Lupine",
        dark: false,
        bg: "#fafafa",
        fg: "#212121",
        fg_dim: "#424242",
        fg_muted: "#757575",
        accent: "#3264eb",
        accent2: "#3264eb",
        highlight: "#026fde",
        special: "#8a4ad7",
        teal: "#0c67de",
        error: "#c900c4",
        border: "#f5f5f5",
    },
    Theme {
        slug: "matte-black",
        name: "Matte Black",
        dark: true,
        bg: "#121212",
        fg: "#bebebe",
        fg_dim: "#8a8a8d",
        fg_muted: "#555555",
        accent: "#e68e0d",
        accent2: "#e68e0d",
        highlight: "#b91c1c",
        special: "#d35f5f",
        teal: "#bebebe",
        error: "#d35f5f",
        border: "#1e1e1e",
    },
    Theme {
        slug: "miasma",
        name: "Miasma",
        dark: true,
        bg: "#222222",
        fg: "#c2c2b0",
        fg_dim: "#8a8a7e",
        fg_muted: "#555555",
        accent: "#78824b",
        accent2: "#78824b",
        highlight: "#b36d43",
        special: "#bb7744",
        teal: "#c9a554",
        error: "#685742",
        border: "#2c2c2c",
    },
    Theme {
        slug: "midnight",
        name: "Midnight",
        dark: true,
        bg: "#000000",
        fg: "#efefef",
        fg_dim: "#efefef",
        fg_muted: "#8a8a8d",
        accent: "#407e70",
        accent2: "#407e70",
        highlight: "#f59e0b",
        special: "#8a8a8d",
        teal: "#407e70",
        error: "#b91c1c",
        border: "#8a8a8d",
    },
    Theme {
        slug: "monokai",
        name: "Monokai",
        dark: true,
        bg: "#0a1220",
        fg: "#f0f2f5",
        fg_dim: "#f0f2f5",
        fg_muted: "#555c65",
        accent: "#b2d977",
        accent2: "#78dce8",
        highlight: "#f2c063",
        special: "#aba0f2",
        teal: "#f28b66",
        error: "#f25e86",
        border: "#555c65",
    },
    Theme {
        slug: "nord",
        name: "Nord",
        dark: true,
        bg: "#2e3440",
        fg: "#d8dee9",
        fg_dim: "#adb5c4",
        fg_muted: "#667080",
        accent: "#81a1c1",
        accent2: "#81a1c1",
        highlight: "#ebcb8b",
        special: "#b48ead",
        teal: "#88c0d0",
        error: "#bf616a",
        border: "#3b4252",
    },
    Theme {
        slug: "one-dark-pro",
        name: "One Dark Pro",
        dark: true,
        bg: "#1e2229",
        fg: "#abb2bf",
        fg_dim: "#c8ccd4",
        fg_muted: "#5c6370",
        accent: "#98c379",
        accent2: "#61afef",
        highlight: "#e5c07b",
        special: "#c678dd",
        teal: "#56b6c2",
        error: "#e06c75",
        border: "#5c6370",
    },
    Theme {
        slug: "osaka-jade",
        name: "Osaka Jade",
        dark: true,
        bg: "#111c18",
        fg: "#c1c497",
        fg_dim: "#d6d5bc",
        fg_muted: "#81b8a8",
        accent: "#509475",
        accent2: "#509475",
        highlight: "#459451",
        special: "#d2689c",
        teal: "#2dd5b7",
        error: "#ff5345",
        border: "#23372b",
    },
    Theme {
        slug: "retro-82",
        name: "Retro 82",
        dark: true,
        bg: "#05182e",
        fg: "#f6dcac",
        fg_dim: "#a7c9c6",
        fg_muted: "#3f8f8a",
        accent: "#faa968",
        accent2: "#3f8f8a",
        highlight: "#e97b3c",
        special: "#3f8f8a",
        teal: "#8cbfb8",
        error: "#f85525",
        border: "#0a2540",
    },
    Theme {
        slug: "ristretto",
        name: "Ristretto",
        dark: true,
        bg: "#2c2525",
        fg: "#e6d9db",
        fg_dim: "#c3b7b8",
        fg_muted: "#72696a",
        accent: "#f38d70",
        accent2: "#f38d70",
        highlight: "#f9cc6c",
        special: "#a8a9eb",
        teal: "#85dacc",
        error: "#fd6883",
        border: "#3d2f2a",
    },
    Theme {
        slug: "rose-pine",
        name: "Rose Pine",
        dark: false,
        bg: "#faf4ed",
        fg: "#575279",
        fg_dim: "#6e6a86",
        fg_muted: "#9893a5",
        accent: "#56949f",
        accent2: "#56949f",
        highlight: "#ea9d34",
        special: "#907aa9",
        teal: "#d7827e",
        error: "#b4637a",
        border: "#f2e9e1",
    },
    Theme {
        slug: "rose-pine-dark",
        name: "Rose Pine Dark",
        dark: true,
        bg: "#191724",
        fg: "#e0def4",
        fg_dim: "#e0def4",
        fg_muted: "#6e6a86",
        accent: "#31748f",
        accent2: "#9ccfd8",
        highlight: "#f6c177",
        special: "#c4a7e7",
        teal: "#ebbcba",
        error: "#eb6f92",
        border: "#6e6a86",
    },
    Theme {
        slug: "solitude",
        name: "Solitude",
        dark: true,
        bg: "#101315",
        fg: "#cacccc",
        fg_dim: "#cbc2be",
        fg_muted: "#4b4e55",
        accent: "#798186",
        accent2: "#798186",
        highlight: "#d9dbdc",
        special: "#aeaeae",
        teal: "#707070",
        error: "#565d60",
        border: "#101315",
    },
    Theme {
        slug: "space-monkey",
        name: "Space Monkey",
        dark: true,
        bg: "#1c0e00",
        fg: "#e7aa5a",
        fg_dim: "#f1e5e7",
        fg_muted: "#948a8b",
        accent: "#bd4924",
        accent2: "#66a891",
        highlight: "#f9cc6c",
        special: "#a8a9eb",
        teal: "#bd4924",
        error: "#fd6883",
        border: "#948a8b",
    },
    Theme {
        slug: "tokyo-night",
        name: "Tokyo Night",
        dark: true,
        bg: "#1a1b26",
        fg: "#a9b1d6",
        fg_dim: "#b4bee6",
        fg_muted: "#565f89",
        accent: "#7aa2f7",
        accent2: "#7aa2f7",
        highlight: "#e0af68",
        special: "#ad8ee6",
        teal: "#449dab",
        error: "#f7768e",
        border: "#24283b",
    },
    Theme {
        slug: "tokyoled",
        name: "Tokyoled",
        dark: true,
        bg: "#000000",
        fg: "#a9b1d6",
        fg_dim: "#acb0d0",
        fg_muted: "#444b6a",
        accent: "#9ece6a",
        accent2: "#7aa2f7",
        highlight: "#e0af68",
        special: "#ad8ee6",
        teal: "#449dab",
        error: "#f7768e",
        border: "#444b6a",
    },
    Theme {
        slug: "vantablack",
        name: "Vantablack",
        dark: true,
        bg: "#000000",
        fg: "#ffffff",
        fg_dim: "#ececec",
        fg_muted: "#505050",
        accent: "#8d8d8d",
        accent2: "#8d8d8d",
        highlight: "#cecece",
        special: "#9b9b9b",
        teal: "#b0b0b0",
        error: "#a4a4a4",
        border: "#1a1a1a",
    },
    Theme {
        slug: "white",
        name: "White",
        dark: false,
        bg: "#ffffff",
        fg: "#000000",
        fg_dim: "#000000",
        fg_muted: "#c0c0c0",
        accent: "#6e6e6e",
        accent2: "#1a1a1a",
        highlight: "#4a4a4a",
        special: "#2e2e2e",
        teal: "#3e3e3e",
        error: "#2a2a2a",
        border: "#c0c0c0",
    },
];

impl Theme {
    pub fn get(slug: &str) -> Option<&'static Theme> {
        THEMES.iter().find(|theme| theme.slug == slug)
    }

    pub fn fallback() -> &'static Theme {
        Self::get(DEFAULT_SLUG).expect("retro-82 is bundled")
    }

    pub fn from_headers(headers: &HeaderMap) -> &'static Theme {
        cookie_slug(headers)
            .and_then(Self::get)
            .unwrap_or_else(Self::fallback)
    }

    pub fn color_scheme(self) -> &'static str {
        if self.dark { "dark" } else { "light" }
    }

    pub fn root_css(self) -> String {
        let accent_dim = format!("{}18", self.accent);
        format!(
            ":root{{color-scheme:{};--bg:{};--fg:{};--fg-dim:{};--fg-muted:{};--accent:{};--accent-dim:{};--accent2:{};--highlight:{};--special:{};--teal:{};--error:{};--border:{}}}",
            self.color_scheme(),
            self.bg,
            self.fg,
            self.fg_dim,
            self.fg_muted,
            self.accent,
            accent_dim,
            self.accent2,
            self.highlight,
            self.special,
            self.teal,
            self.error,
            self.border
        )
    }

    pub fn index(self) -> usize {
        THEMES
            .iter()
            .position(|theme| theme.slug == self.slug)
            .unwrap_or(0)
    }

    pub fn prev(self) -> &'static Theme {
        let i = self.index();
        &THEMES[(i + THEMES.len() - 1) % THEMES.len()]
    }

    pub fn next(self) -> &'static Theme {
        let i = self.index();
        &THEMES[(i + 1) % THEMES.len()]
    }

    pub fn preview_style(self) -> String {
        format!(
            "--bg:{bg};--fg:{fg};--fg-dim:{dim};--fg-muted:{muted};--accent:{accent};--accent-dim:{accent}18;--accent2:{a2};--highlight:{hi};--special:{sp};--teal:{teal};--error:{err};--border:{border};background:{bg};color:{fg};border-color:{accent}",
            bg = self.bg,
            fg = self.fg,
            dim = self.fg_dim,
            muted = self.fg_muted,
            accent = self.accent,
            a2 = self.accent2,
            hi = self.highlight,
            sp = self.special,
            teal = self.teal,
            err = self.error,
            border = self.border
        )
    }

    pub fn favicon_svg(self) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" rx="8" fill="{bg}"/><text x="50" y="42" text-anchor="middle" font-family="monospace" font-size="38" font-weight="700" fill="{accent}">HL</text><text x="50" y="82" text-anchor="middle" font-family="monospace" font-size="38" font-weight="700" fill="{accent}">DR</text></svg>"#,
            bg = self.bg,
            accent = self.accent
        )
    }

    pub fn cookie_header(self) -> String {
        format!(
            "{COOKIE}={}; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly",
            self.slug
        )
    }
}

fn cookie_slug(headers: &HeaderMap) -> Option<&str> {
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

    #[test]
    fn default_is_retro_82() {
        assert_eq!(Theme::fallback().slug, "retro-82");
        assert_eq!(Theme::fallback().name, "Retro 82");
        assert!(Theme::fallback().dark);
    }

    #[test]
    fn slugs_are_unique_and_lookup_works() {
        let mut seen = std::collections::HashSet::new();
        for theme in THEMES {
            assert!(seen.insert(theme.slug), "{}", theme.slug);
            assert_eq!(Theme::get(theme.slug).unwrap().name, theme.name);
        }
        assert!(Theme::get("not-a-theme").is_none());
        assert_eq!(THEMES.len(), 33);
    }

    #[test]
    fn cookie_selects_theme_and_unknown_falls_back() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=tokyo-night"),
        );
        assert_eq!(Theme::from_headers(&headers).slug, "tokyo-night");
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=nope; other=1"),
        );
        assert_eq!(Theme::from_headers(&headers).slug, "retro-82");
        assert_eq!(Theme::from_headers(&HeaderMap::new()).slug, "retro-82");
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
        let css = Theme::fallback().root_css();
        assert!(css.contains("--bg:#05182e"));
        assert!(css.contains("--accent:#faa968"));
        assert!(css.contains("color-scheme:dark"));
    }

    #[test]
    fn favicon_uses_theme_colors() {
        let svg = Theme::fallback().favicon_svg();
        assert!(svg.contains("fill=\"#05182e\""));
        assert!(svg.contains("fill=\"#faa968\""));
        assert!(svg.contains(">HL</text>"));
    }

    #[test]
    fn neighbors_wrap_around() {
        let first = THEMES[0];
        let last = THEMES[THEMES.len() - 1];
        assert_eq!(first.prev().slug, last.slug);
        assert_eq!(last.next().slug, first.slug);
        let current = Theme::fallback();
        assert_ne!(current.prev().slug, current.slug);
        assert_ne!(current.next().slug, current.slug);
    }
}
