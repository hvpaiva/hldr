use serde::Serialize;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

impl Health {
    pub fn ok() -> Self {
        Self {
            status: "ok",
            version: VERSION,
        }
    }
}
