use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use tokio::net::UnixListener;

/// Where the private API listens: `127.0.0.1:8081` or `unix:/run/tailnet/8443.sock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenAddr {
    Tcp(SocketAddr),
    Unix(PathBuf),
}

impl FromStr for ListenAddr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(path) = s.strip_prefix("unix:") {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(format!("unix socket path must be absolute: {s}"));
            }
            return Ok(Self::Unix(path));
        }
        s.parse()
            .map(Self::Tcp)
            .map_err(|_| format!("expected host:port or unix:/path, got {s}"))
    }
}

impl fmt::Display for ListenAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "{addr}"),
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
        }
    }
}

/// Binds a unix socket, taking the path over from a previous process.
///
/// During a deploy the old container still listens on the path while the
/// new one boots. Unlinking and binding hands new connections to the new
/// process; the old one serves what it already accepted until it stops.
/// Anything at the path that is not a socket is left alone.
pub fn bind_unix(path: &Path) -> io::Result<UnixListener> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(path)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    UnixListener::bind(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_forms() {
        assert_eq!(
            "127.0.0.1:8081".parse::<ListenAddr>().unwrap(),
            ListenAddr::Tcp("127.0.0.1:8081".parse().unwrap())
        );
        assert_eq!(
            "unix:/run/tailnet/8443.sock".parse::<ListenAddr>().unwrap(),
            ListenAddr::Unix("/run/tailnet/8443.sock".into())
        );
        assert!("unix:relative.sock".parse::<ListenAddr>().is_err());
        assert!("localhost".parse::<ListenAddr>().is_err());
    }

    #[tokio::test]
    async fn takes_over_a_socket_left_by_another_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.sock");
        let old = bind_unix(&path).unwrap();
        let new = bind_unix(&path).unwrap();

        let _client = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (_conn, _) = new.accept().await.unwrap();
        drop(old);
    }

    #[test]
    fn refuses_to_replace_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.sock");
        std::fs::write(&path, "data").unwrap();
        let err = bind_unix(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "data");
    }
}
