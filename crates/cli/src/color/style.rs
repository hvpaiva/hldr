//! One color spec, in kubecolor's syntax: `fields:joined:by:colons`, each a
//! color name, an attribute, `fg=COLOR`, `bg=COLOR`, a 256-color code, a hex
//! code or `rgb(r, g, b)`. `none` alone means no styling.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::builder::styling;

/// How many colors the terminal shows; a color it cannot show is drawn as
/// the nearest one it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Basic,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    /// The terminal's default color.
    Default,
    /// One of the 16 ANSI colors, 8 to 15 being the bright ones.
    Basic(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Code {
    /// An SGR attribute, such as 1 for bold.
    Attr(u8),
    Fg(Color),
    Bg(Color),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    codes: Vec<Code>,
}

pub static PLAIN: Style = Style { codes: Vec::new() };

impl Style {
    pub fn parse(spec: &str) -> Result<Self> {
        let mut codes = Vec::new();
        for field in spec.split(':').map(str::trim) {
            if field.is_empty() || field.eq_ignore_ascii_case("none") {
                continue;
            }
            codes.push(match field.split_once('=') {
                Some((key, value)) => match key.trim() {
                    "fg" => Code::Fg(color(value.trim())?),
                    "bg" => Code::Bg(color(value.trim())?),
                    other => bail!("invalid color key {:?} in {spec:?}", format!("{other}=")),
                },
                None => match attribute(field) {
                    Some(attr) => Code::Attr(attr),
                    None => Code::Fg(color(field)?),
                },
            });
        }
        Ok(Self { codes })
    }

    /// Several styles separated by `/`, such as `hicyan / cyan`.
    pub fn parse_list(spec: &str) -> Result<Vec<Self>> {
        spec.split('/').map(Self::parse).collect()
    }

    pub fn is_plain(&self) -> bool {
        self.codes.is_empty()
    }

    /// The SGR parameters, such as `1;31`, for a terminal at `level`.
    pub fn sgr(&self, level: Level) -> String {
        let mut out = String::new();
        for code in &self.codes {
            if !out.is_empty() {
                out.push(';');
            }
            match *code {
                Code::Attr(attr) => {
                    let _ = write!(out, "{attr}");
                }
                Code::Fg(color) => push_color(&mut out, color, level, false),
                Code::Bg(color) => push_color(&mut out, color, level, true),
            }
        }
        out
    }

    /// The same style as clap draws help with.
    pub fn clap(&self, level: Level) -> styling::Style {
        let color = |color| match downgrade(color, level) {
            Color::Default => None,
            Color::Basic(i) => Some(styling::Color::Ansi(BASIC_ANSI[usize::from(i)])),
            Color::Indexed(i) => Some(styling::Color::Ansi256(styling::Ansi256Color(i))),
            Color::Rgb(r, g, b) => Some(styling::Color::Rgb(styling::RgbColor(r, g, b))),
        };
        self.codes
            .iter()
            .fold(styling::Style::new(), |style, code| match *code {
                Code::Fg(c) => style.fg_color(color(c)),
                Code::Bg(c) => style.bg_color(color(c)),
                Code::Attr(attr) => style.effects(style.get_effects() | effect(attr)),
            })
    }
}

const BASIC_ANSI: [styling::AnsiColor; 16] = {
    use styling::AnsiColor::*;
    [
        Black,
        Red,
        Green,
        Yellow,
        Blue,
        Magenta,
        Cyan,
        White,
        BrightBlack,
        BrightRed,
        BrightGreen,
        BrightYellow,
        BrightBlue,
        BrightMagenta,
        BrightCyan,
        BrightWhite,
    ]
};

fn effect(attr: u8) -> styling::Effects {
    match attr {
        1 => styling::Effects::BOLD,
        2 => styling::Effects::DIMMED,
        3 => styling::Effects::ITALIC,
        4 => styling::Effects::UNDERLINE,
        5 | 6 => styling::Effects::BLINK,
        7 => styling::Effects::INVERT,
        8 => styling::Effects::HIDDEN,
        9 => styling::Effects::STRIKETHROUGH,
        _ => styling::Effects::new(),
    }
}

fn attribute(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "reset" => 0,
        "bold" | "b" => 1,
        "fuzzy" | "faint" | "dim" => 2,
        "italic" | "i" => 3,
        "underscore" | "underscored" | "underline" | "underlined" | "u" => 4,
        "blink" => 5,
        "fastblink" => 6,
        "reverse" | "invert" | "inverted" => 7,
        "concealed" | "hidden" | "invisible" => 8,
        "strikethrough" | "strike" => 9,
        _ => return None,
    })
}

fn color(spec: &str) -> Result<Color> {
    if let Some(color) = named(spec) {
        return Ok(color);
    }
    if let Some(inner) = spec
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return rgb(inner).ok_or_else(|| anyhow::anyhow!("invalid rgb color {spec:?}"));
    }
    if spec.matches(',').count() == 2
        && let Some(color) = rgb(spec)
    {
        return Ok(color);
    }
    if spec.len() < 4
        && spec.bytes().all(|b| b.is_ascii_digit())
        && let Ok(code) = spec.parse::<u8>()
    {
        return Ok(Color::Indexed(code));
    }
    if let Some(color) = hex(spec) {
        return Ok(color);
    }
    bail!("invalid color {spec:?}: use a name, 0-255, #rrggbb or rgb(r, g, b)")
}

fn named(spec: &str) -> Option<Color> {
    let name = spec
        .to_ascii_lowercase()
        .replace("hi_", "hi")
        .replace("hi-", "hi")
        .replace("light_", "light")
        .replace("light-", "light");
    let (bright, base) = match name
        .strip_prefix("hi")
        .or_else(|| name.strip_prefix("light"))
    {
        Some(base) => (true, base.to_owned()),
        None => (false, name.clone()),
    };
    let index = match base.as_str() {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" | "brown" => 3,
        "blue" => 4,
        "magenta" | "purple" => 5,
        "cyan" => 6,
        "white" => 7,
        "default" | "normal" | "transparent" if !bright => return Some(Color::Default),
        _ => {
            return match name.as_str() {
                "gray" | "grey" | "darkgray" | "darkgrey" => Some(Color::Basic(8)),
                "lime" => Some(Color::Basic(10)),
                "gold" => Some(Color::Basic(11)),
                _ => None,
            };
        }
    };
    Some(Color::Basic(if bright { index + 8 } else { index }))
}

fn rgb(spec: &str) -> Option<Color> {
    let parts: Vec<u8> = spec
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b] => Some(Color::Rgb(r, g, b)),
        _ => None,
    }
}

fn hex(spec: &str) -> Option<Color> {
    let digits = spec
        .strip_prefix('#')
        .or_else(|| spec.strip_prefix("0x"))
        .unwrap_or(spec);
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |text: &str| u8::from_str_radix(text, 16).ok();
    match digits.len() {
        3 => {
            let mut doubled = digits.chars().map(|c| channel(&format!("{c}{c}")));
            Some(Color::Rgb(
                doubled.next()??,
                doubled.next()??,
                doubled.next()??,
            ))
        }
        6 => Some(Color::Rgb(
            channel(&digits[0..2])?,
            channel(&digits[2..4])?,
            channel(&digits[4..6])?,
        )),
        _ => None,
    }
}

/// The nearest color a terminal at `level` shows.
fn downgrade(color: Color, level: Level) -> Color {
    match (color, level) {
        (Color::Rgb(r, g, b), Level::Ansi256) => Color::Indexed(rgb_to_256(r, g, b)),
        (Color::Rgb(r, g, b), Level::Basic) => Color::Basic(nearest_basic(r, g, b)),
        (Color::Indexed(i), Level::Basic) if i >= 16 => {
            let (r, g, b) = indexed_to_rgb(i);
            Color::Basic(nearest_basic(r, g, b))
        }
        (Color::Indexed(i), Level::Basic) => Color::Basic(i),
        (other, _) => other,
    }
}

fn push_color(out: &mut String, color: Color, level: Level, background: bool) {
    let base = if background { 40 } else { 30 };
    let _ = match downgrade(color, level) {
        Color::Default => write!(out, "{}", base + 9),
        Color::Basic(i) if i < 8 => write!(out, "{}", base + i),
        Color::Basic(i) => write!(out, "{}", base + 60 + (i - 8)),
        Color::Indexed(i) => write!(out, "{};5;{i}", base + 8),
        Color::Rgb(r, g, b) => write!(out, "{};2;{r};{g};{b}", base + 8),
    };
}

/// xterm's values for the 16 ANSI colors.
const BASIC: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        return match r {
            0..8 => 16,
            249.. => 231,
            // 24 grays from 8 to 238, ten apart.
            _ => 232 + ((r - 3) / 10).min(23),
        };
    }
    let step = |c: u8| ((u16::from(c) * 5 + 127) / 255) as u8;
    16 + 36 * step(r) + 6 * step(g) + step(b)
}

fn indexed_to_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0..16 => BASIC[usize::from(index)],
        16..232 => {
            let i = index - 16;
            (
                CUBE[usize::from(i / 36)],
                CUBE[usize::from(i / 6 % 6)],
                CUBE[usize::from(i % 6)],
            )
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

fn nearest_basic(r: u8, g: u8, b: u8) -> u8 {
    let distance = |&(br, bg, bb): &(u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(r, br) + d(g, bg) + d(b, bb)
    };
    BASIC
        .iter()
        .enumerate()
        .min_by_key(|(_, c)| distance(c))
        .map_or(7, |(i, _)| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sgr(spec: &str, level: Level) -> String {
        Style::parse(spec).unwrap().sgr(level)
    }

    #[test]
    fn parses_kubecolor_syntax() {
        assert_eq!(sgr("red", Level::TrueColor), "31");
        assert_eq!(sgr("hicyan", Level::TrueColor), "96");
        assert_eq!(sgr("hi_cyan", Level::TrueColor), "96");
        assert_eq!(sgr("light-blue", Level::TrueColor), "94");
        assert_eq!(sgr("gray:italic", Level::TrueColor), "90;3");
        assert_eq!(sgr("fg=white:bg=red:bold", Level::TrueColor), "37;41;1");
        assert_eq!(sgr("208", Level::TrueColor), "38;5;208");
        assert_eq!(sgr("#feb927", Level::TrueColor), "38;2;254;185;39");
        assert_eq!(sgr("fg=#fff", Level::TrueColor), "38;2;255;255;255");
        assert_eq!(sgr("bg=rgb(1, 2, 3)", Level::TrueColor), "48;2;1;2;3");
        assert_eq!(sgr("10,20,30", Level::TrueColor), "38;2;10;20;30");
        assert_eq!(sgr("default", Level::TrueColor), "39");
        assert!(Style::parse("none").unwrap().is_plain());
        assert!(Style::parse("").unwrap().is_plain());
    }

    #[test]
    fn rejects_what_is_not_a_color() {
        for bad in [
            "reddish",
            "fg=",
            "xx=red",
            "#12345",
            "rgb(1,2)",
            "rgb(300,0,0)",
        ] {
            assert!(Style::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lists_split_on_slashes() {
        let list = Style::parse_list("hicyan / cyan").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].sgr(Level::Basic), "36");
    }

    #[test]
    fn converts_to_clap_styles() {
        let style = Style::parse("fg=#ff0000:bg=4:bold:underline").unwrap();
        assert_eq!(
            style.clap(Level::Basic).render().to_string(),
            styling::Style::new()
                .fg_color(Some(styling::AnsiColor::BrightRed.into()))
                .bg_color(Some(styling::AnsiColor::Blue.into()))
                .bold()
                .underline()
                .render()
                .to_string()
        );
        assert_eq!(
            Style::parse("208").unwrap().clap(Level::TrueColor),
            styling::Style::new().fg_color(Some(styling::Ansi256Color(208).into()))
        );
        assert_eq!(PLAIN.clap(Level::TrueColor), styling::Style::new());
    }

    #[test]
    fn downgrades_to_what_the_terminal_shows() {
        assert_eq!(sgr("#ff0000", Level::Ansi256), "38;5;196");
        assert_eq!(sgr("#ff0000", Level::Basic), "91");
        assert_eq!(sgr("#808080", Level::Ansi256), "38;5;244");
        assert_eq!(sgr("196", Level::Basic), "91");
        assert_eq!(sgr("4", Level::Basic), "34");
        assert_eq!(sgr("bg=#c2270a", Level::Basic), "41");
    }
}
