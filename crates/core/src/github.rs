//! What GitHub's refusals mean, for the server and the CLI alike.

/// GitHub refused a request because a rate limit ran out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    /// When requests are accepted again, in seconds since the Unix epoch.
    pub reset: u64,
}

impl RateLimit {
    /// Reads an answer's status and headers. The primary limit answers 403
    /// or 429 with `x-ratelimit-remaining: 0` and the reset time; a secondary
    /// limit sends `retry-after` in seconds instead. `now` is seconds since
    /// the Unix epoch.
    pub fn from_answer(
        status: u16,
        remaining: Option<&str>,
        reset: Option<&str>,
        retry_after: Option<&str>,
        now: u64,
    ) -> Option<Self> {
        if !matches!(status, 403 | 429) {
            return None;
        }
        let number = |value: Option<&str>| value.and_then(|v| v.trim().parse::<u64>().ok());
        if let Some(wait) = number(retry_after) {
            return Some(Self {
                reset: now.saturating_add(wait),
            });
        }
        if number(remaining) == Some(0) {
            return Some(Self {
                reset: number(reset).unwrap_or(now),
            });
        }
        None
    }

    /// Such as `GitHub rate limit exceeded; it resets at 21:14:05 UTC, in 12 min`.
    pub fn message(&self, now: u64) -> String {
        let clock = self.reset % 86_400;
        let wait = self.reset.saturating_sub(now);
        let within = if wait < 60 {
            format!("{wait}s")
        } else {
            format!("{} min", wait.div_ceil(60))
        };
        format!(
            "GitHub rate limit exceeded; it resets at {:02}:{:02}:{:02} UTC, in {within}",
            clock / 3600,
            clock % 3600 / 60,
            clock % 60
        )
    }
}

/// Seconds since the Unix epoch, as the rate limit headers count them.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000;

    #[test]
    fn reads_the_primary_limit() {
        let limit = RateLimit::from_answer(403, Some("0"), Some("1790000725"), None, NOW).unwrap();
        assert_eq!(limit.reset, 1_790_000_725);
        assert_eq!(
            limit.message(NOW),
            "GitHub rate limit exceeded; it resets at 14:25:25 UTC, in 13 min"
        );
    }

    #[test]
    fn reads_the_secondary_limit() {
        let limit = RateLimit::from_answer(429, None, None, Some("30"), NOW).unwrap();
        assert_eq!(limit.reset, NOW + 30);
        assert!(limit.message(NOW).ends_with("in 30s"));
    }

    #[test]
    fn other_refusals_are_not_limits() {
        assert_eq!(
            RateLimit::from_answer(403, Some("12"), Some("1"), None, NOW),
            None
        );
        assert_eq!(RateLimit::from_answer(403, None, None, None, NOW), None);
        assert_eq!(
            RateLimit::from_answer(404, Some("0"), Some("1"), None, NOW),
            None
        );
    }
}
