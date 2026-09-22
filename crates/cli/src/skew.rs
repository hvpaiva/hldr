//! The version skew policy, as Kubernetes has it between kubectl and the API
//! server: the client and the server ship from the same release, and a
//! client supports a server of the same major whose minor is at most one
//! away. Patches never matter. Compatibility itself comes from the API
//! policy (see the README): within a major, the API only grows.
//!
//! Before 1.0, as Cargo reads versions, `0.y` is the major: a breaking
//! change takes the minor, so client and server must share it. From 1.0 on
//! the rule above applies as it is; the move to 1.0 changes no code.

/// Where a client and a server stand against the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skew {
    /// Within the policy, or a side is not a release (`dev`, unknown).
    Supported,
    /// Same major, minors more than one apart: likely to work, never tested.
    Beyond,
    /// Across a breaking release, where the API may have broken.
    Unsupported,
}

pub fn skew(client: &str, server: &str) -> Skew {
    let (Some(client), Some(server)) = (release(client), release(server)) else {
        return Skew::Supported;
    };
    if breaking_line(client) != breaking_line(server) {
        Skew::Unsupported
    } else if client.1.abs_diff(server.1) > 1 {
        Skew::Beyond
    } else {
        Skew::Supported
    }
}

/// What a breaking change moves: the major, or before 1.0 the minor.
fn breaking_line((major, minor): (u64, u64)) -> (u64, u64) {
    if major == 0 { (0, minor) } else { (major, 0) }
}

/// What to tell the user, or `None` within the policy.
pub fn explain(client: &str, server: &str) -> Option<String> {
    let problem = match skew(client, server) {
        Skew::Supported => return None,
        Skew::Beyond => "are more than one minor release apart, beyond the supported skew",
        Skew::Unsupported => {
            "are on either side of a breaking release, and may not speak the same API"
        }
    };
    Some(format!(
        "client {client} and server {server} {problem}; install the hldr released with the \
         server: {INSTALL} -s -- --version {server}"
    ))
}

/// The installer's one line; `-s -- --version X.Y.Z` picks a release.
pub const INSTALL: &str = "curl -fsSL https://hvpaiva.dev/install.sh | bash";

/// Major and minor of an `X.Y.Z` release; `None` for anything else.
fn release(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let _patch: u64 = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_kubernetes_policy() {
        for (client, server, expected) in [
            ("4.6.0", "4.6.0", Skew::Supported),
            ("4.6.0", "4.6.9", Skew::Supported),
            ("4.6.3", "4.5.0", Skew::Supported),
            ("4.5.0", "4.6.3", Skew::Supported),
            ("4.4.0", "4.6.0", Skew::Beyond),
            ("4.8.1", "4.6.0", Skew::Beyond),
            ("5.0.0", "4.9.0", Skew::Unsupported),
            ("4.6.0", "5.6.0", Skew::Unsupported),
            ("dev", "4.6.0", Skew::Supported),
            ("4.6.0", "dev", Skew::Supported),
            ("4.6.0", "unknown", Skew::Supported),
            ("4.6", "4.9.0", Skew::Supported),
            ("4.6.0.1", "4.9.0", Skew::Supported),
            // Before 1.0, the minor is the major.
            ("0.3.0", "0.3.9", Skew::Supported),
            ("0.3.4", "0.3.0", Skew::Supported),
            ("0.3.0", "0.4.0", Skew::Unsupported),
            ("0.4.2", "0.3.2", Skew::Unsupported),
            ("0.9.0", "1.0.0", Skew::Unsupported),
            ("1.0.0", "1.1.0", Skew::Supported),
        ] {
            assert_eq!(skew(client, server), expected, "{client} against {server}");
        }
    }

    #[test]
    fn explains_only_what_is_outside_the_policy() {
        assert_eq!(explain("4.6.0", "4.5.2"), None);
        let beyond = explain("4.3.0", "4.6.1").unwrap();
        assert!(
            beyond.starts_with("client 4.3.0 and server 4.6.1 are more than one minor"),
            "{beyond}"
        );
        assert!(
            beyond.ends_with("install.sh | bash -s -- --version 4.6.1"),
            "{beyond}"
        );
        let major = explain("5.0.0", "4.6.1").unwrap();
        assert!(
            major.contains("either side of a breaking release"),
            "{major}"
        );
        let early = explain("0.2.0", "0.3.1").unwrap();
        assert!(
            early.contains("either side of a breaking release"),
            "{early}"
        );
    }
}
