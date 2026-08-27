//! Structured diagnostics for the engine: log subscriber setup and the
//! slow-statement threshold.
//!
//! The engine is a subprocess of the client library, so stdout carries the
//! JSON-RPC stream and every diagnostic goes to stderr. Both knobs are read
//! from the environment because the engine is spawned by generated clients
//! that pass no logging flags of their own.

use std::io::IsTerminal;
use std::time::{Duration, Instant};

use tracing_subscriber::filter::EnvFilter;

/// Environment variable holding `tracing` filter directives
/// (e.g. `nautilus_engine=debug`). Falls back to `RUST_LOG`.
pub const LOG_FILTER_ENV: &str = "NAUTILUS_LOG";

/// Environment variable holding the slow-statement threshold in milliseconds.
/// Unset — or `0` — disables slow-statement logging.
pub const SLOW_QUERY_ENV: &str = "NAUTILUS_SLOW_QUERY_MS";

/// Filter applied when neither [`LOG_FILTER_ENV`] nor `RUST_LOG` is set.
///
/// Scoped to this crate: `sqlx` emits a record per executed statement, which
/// at a bare `info` default would duplicate the whole query stream onto
/// stderr.
const DEFAULT_LOG_FILTER: &str = "nautilus_engine=info";

/// `tracing` target carrying slow-statement records, so operators can route
/// them separately (`NAUTILUS_LOG=nautilus_engine=warn`, or
/// `nautilus_engine::slow_query=warn` alone).
pub(crate) const SLOW_QUERY_TARGET: &str = "nautilus_engine::slow_query";

/// Install the process-wide log subscriber writing to stderr.
///
/// Idempotent: a subscriber installed by an embedding application wins and
/// this call becomes a no-op, so the engine never steals logging from a host
/// process that links it as a library.
pub fn init() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(resolve_filter(
            std::env::var(LOG_FILTER_ENV).ok().as_deref(),
            std::env::var("RUST_LOG").ok().as_deref(),
        ))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .try_init();
}

/// Slow-statement threshold configured for this process, or `None` when
/// slow-statement logging is disabled.
pub fn slow_query_threshold() -> Option<Duration> {
    parse_slow_query_threshold(std::env::var(SLOW_QUERY_ENV).ok().as_deref())
}

/// Build the filter from the two supported variables, falling back to
/// [`DEFAULT_LOG_FILTER`] when both are absent or hold invalid directives.
fn resolve_filter(nautilus_log: Option<&str>, rust_log: Option<&str>) -> EnvFilter {
    nautilus_log
        .or(rust_log)
        .and_then(|directives| EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| EnvFilter::new(DEFAULT_LOG_FILTER))
}

/// Parse the millisecond threshold, treating an unparsable or zero value as
/// "disabled" rather than an error: a mistyped variable must not stop an
/// engine the client already spawned.
fn parse_slow_query_threshold(raw: Option<&str>) -> Option<Duration> {
    raw?.trim()
        .parse::<u64>()
        .ok()
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
}

/// Times one executed statement and emits a slow-statement record when it runs
/// past the configured threshold.
///
/// A disabled threshold skips the clock read entirely, so the default
/// configuration costs the hot path nothing beyond moving an `Option`.
pub(crate) struct StatementTimer<'a> {
    started: Option<(Instant, Duration)>,
    context: &'a str,
    sql: &'a str,
}

impl<'a> StatementTimer<'a> {
    /// Begin timing a statement. `context` is the operation tag already
    /// carried by the execution paths (`Query`, `Count`, `rawQuery`, ...).
    pub(crate) fn start(threshold: Option<Duration>, context: &'a str, sql: &'a str) -> Self {
        Self {
            started: threshold.map(|threshold| (Instant::now(), threshold)),
            context,
            sql,
        }
    }

    /// Emit the slow-statement record if the statement ran past the threshold.
    pub(crate) fn finish(self) {
        let Some((started, threshold)) = self.started else {
            return;
        };
        let elapsed = started.elapsed();
        if elapsed < threshold {
            return;
        }
        tracing::warn!(
            target: SLOW_QUERY_TARGET,
            context = self.context,
            elapsed_ms = elapsed.as_millis() as u64,
            sql = self.sql,
            "slow statement"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Writer that keeps emitted log lines in memory for assertions.
    #[derive(Clone, Default)]
    struct CapturedLog(Arc<Mutex<Vec<u8>>>);

    impl CapturedLog {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().expect("log buffer").clone()).expect("utf-8 log")
        }
    }

    impl Write for CapturedLog {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedLog {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Time a statement that runs for `elapsed` and return what was logged.
    fn log_of_statement(threshold: Option<Duration>, elapsed: Duration) -> String {
        let logs = CapturedLog::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .with_env_filter(EnvFilter::new(DEFAULT_LOG_FILTER))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let timer = StatementTimer::start(threshold, "Query", "SELECT 1");
            std::thread::sleep(elapsed);
            timer.finish();
        });

        logs.contents()
    }

    #[test]
    fn slow_query_threshold_is_disabled_without_a_value() {
        assert_eq!(parse_slow_query_threshold(None), None);
    }

    #[test]
    fn slow_query_threshold_parses_milliseconds() {
        assert_eq!(
            parse_slow_query_threshold(Some("250")),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            parse_slow_query_threshold(Some(" 40 ")),
            Some(Duration::from_millis(40))
        );
    }

    #[test]
    fn slow_query_threshold_treats_zero_and_garbage_as_disabled() {
        assert_eq!(parse_slow_query_threshold(Some("0")), None);
        assert_eq!(parse_slow_query_threshold(Some("fast")), None);
        assert_eq!(parse_slow_query_threshold(Some("-5")), None);
        assert_eq!(parse_slow_query_threshold(Some("")), None);
    }

    #[test]
    fn filter_prefers_nautilus_log_over_rust_log() {
        let filter = resolve_filter(Some("nautilus_engine=debug"), Some("trace"));
        assert_eq!(filter.to_string(), "nautilus_engine=debug");
    }

    #[test]
    fn filter_falls_back_to_rust_log_then_to_the_default() {
        assert_eq!(
            resolve_filter(None, Some("nautilus_engine=trace")).to_string(),
            "nautilus_engine=trace"
        );
        assert_eq!(resolve_filter(None, None).to_string(), DEFAULT_LOG_FILTER);
    }

    #[test]
    fn slow_statements_are_logged_with_their_sql_and_duration() {
        let logged = log_of_statement(Some(Duration::from_millis(1)), Duration::from_millis(20));
        assert!(logged.contains("slow statement"), "{logged}");
        assert!(logged.contains(SLOW_QUERY_TARGET), "{logged}");
        assert!(logged.contains("SELECT 1"), "{logged}");
        assert!(logged.contains("context=\"Query\""), "{logged}");
    }

    #[test]
    fn fast_statements_and_disabled_thresholds_log_nothing() {
        assert!(log_of_statement(Some(Duration::from_secs(60)), Duration::ZERO).is_empty());
        assert!(log_of_statement(None, Duration::from_millis(20)).is_empty());
    }

    #[test]
    fn filter_falls_back_when_directives_are_invalid() {
        assert_eq!(
            resolve_filter(Some("=?!not a directive"), None).to_string(),
            DEFAULT_LOG_FILTER
        );
    }
}
