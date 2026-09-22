use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::client::Client;
use crate::color::Term;
use crate::discovery::Catalog;
use crate::print::{self, Fetched, Output};

/// How often `--watch` asks the server what changed.
const WATCH_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, clap::Args)]
pub struct Args {
    /// TYPE, TYPE NAME..., or TYPE/NAME...
    #[arg(value_name = "TYPE[/NAME]", required = true)]
    targets: Vec<String>,
    /// table, wide, json, yaml, name, jsonpath=TEMPLATE or
    /// custom-columns=HEADER:PATH,...
    #[arg(short, long, default_value = "table")]
    output: String,
    /// Leave out the header row of tables.
    #[arg(long)]
    no_headers: bool,
    /// After listing, print what changes, until interrupted
    #[arg(short, long)]
    watch: bool,
}

impl Args {
    /// A watch runs until interrupted, so its output is never paged.
    pub fn watching(&self) -> bool {
        self.watch
    }
}

pub fn run(
    out: &mut dyn Write,
    term: &Term,
    client: &Client,
    catalog: &mut Catalog<'_>,
    args: &Args,
) -> Result<bool> {
    let output: Output = args.output.parse()?;
    if args.watch {
        watch(
            out,
            term,
            client,
            catalog,
            args,
            &output,
            &Every(WATCH_EVERY),
        )?;
        return Ok(true);
    }
    let (fetched, complete) = super::fetch(term, client, catalog, &args.targets, "get")?;
    print::print(out, term.out(), &fetched, &output, args.no_headers)?;
    Ok(complete)
}

/// How long a watch waits between polls, and whether to go on.
trait Pace {
    fn next(&self) -> bool;
}

struct Every(Duration);

impl Pace for Every {
    fn next(&self) -> bool {
        std::thread::sleep(self.0);
        true
    }
}

/// Lists, then polls for what was written after the last stream position,
/// as `kubectl get -w` streams changes. A failed poll is reported once and
/// retried: a deploy restarting the server does not end the watch.
fn watch(
    out: &mut dyn Write,
    term: &Term,
    client: &Client,
    catalog: &mut Catalog<'_>,
    args: &Args,
    output: &Output,
    pace: &dyn Pace,
) -> Result<()> {
    let [target] = args.targets.as_slice() else {
        bail!("--watch takes one resource type, such as `hldr get events --watch`");
    };
    if target.contains('/') {
        bail!("--watch takes a resource type, not {target:?}");
    }
    let resource = catalog.resolve(target)?;
    if !resource.verbs.iter().any(|verb| verb == "watch") {
        bail!("{} cannot be watched", resource.name);
    }
    let path = format!("/api/v1/{}", resource.name);
    let first = client.get(&path)?;
    let mut seq = position(&first)?;
    let fetched = [Fetched {
        resource: resource.clone(),
        value: first,
    }];
    print::print(out, term.out(), &fetched, output, args.no_headers)?;
    out.flush()?;

    let mut failing = false;
    while pace.next() {
        let update = match client.get(&format!("{path}?after={seq}")) {
            Ok(update) => update,
            Err(err) => {
                if !failing {
                    term.warning(&format!("{err:#}; still watching"));
                }
                failing = true;
                continue;
            }
        };
        failing = false;
        seq = position(&update)?;
        let items: Vec<Value> = print::items(&update).into_iter().cloned().collect();
        if items.is_empty() {
            continue;
        }
        // A table prints the rows of one poll together, without a header
        // again; other formats print each object on its own.
        let batches: Vec<Value> = match output {
            Output::Table | Output::Wide | Output::CustomColumns(_) => vec![update],
            _ => items,
        };
        for value in batches {
            let fetched = [Fetched {
                resource: resource.clone(),
                value,
            }];
            print::print(out, term.out(), &fetched, output, true)?;
        }
        out.flush()?;
    }
    Ok(())
}

/// Where the stream stood when a list was read.
fn position(list: &Value) -> Result<i64> {
    list["seq"]
        .as_i64()
        .context("the server sent a list without a stream position")
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use serde_json::json;

    use super::*;
    use crate::stub::{self, Stub};

    /// Polls `n` times without waiting.
    struct Times(Cell<usize>);

    impl Pace for Times {
        fn next(&self) -> bool {
            let left = self.0.get();
            self.0.set(left.saturating_sub(1));
            left > 0
        }
    }

    fn event(name: &str, seq: i64, event_type: &str, reason: &str, count: u32) -> Value {
        json!({
            "kind": "Event", "metadata": {"name": name, "seq": seq},
            "type": event_type, "reason": reason, "message": format!("{reason} happened"),
            "revision": null, "count": count,
            "first_at": "2026-09-22T10:00:00Z", "last_at": format!("2026-09-22T10:0{seq}:00Z"),
        })
    }

    fn list(seq: i64, items: Vec<Value>) -> Value {
        json!({"kind": "EventList", "seq": seq, "items": items})
    }

    fn args(extra: &[&str]) -> Args {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let mut argv = vec!["get"];
        argv.extend(extra);
        Cli::parse_from(argv).args
    }

    fn watched(stub: &Stub, extra: &[&str], polls: usize) -> (Result<()>, String) {
        let client = Client::new(stub.serve());
        let mut catalog = Catalog::new(&client, None);
        let args = args(extra);
        let output: Output = args.output.parse().unwrap();
        let mut out = Vec::new();
        let result = watch(
            &mut out,
            &Term::plain(),
            &client,
            &mut catalog,
            &args,
            &output,
            &Times(Cell::new(polls)),
        );
        (result, String::from_utf8(out).unwrap())
    }

    fn events_stub() -> Stub {
        let stub = Stub::default();
        let mut events = stub::resource("events", "event", "Event");
        events["verbs"] = json!(["get", "explain", "watch"]);
        events["short_names"] = json!(["ev"]);
        events["columns"] = json!([
            {"name": "TYPE", "json_path": ".type", "wide": false},
            {"name": "REASON", "json_path": ".reason", "wide": false},
            {"name": "COUNT", "json_path": ".count", "wide": false},
        ]);
        stub.extra.lock().unwrap().push(events);
        stub
    }

    #[test]
    fn watch_prints_the_list_then_what_changes() {
        let stub = events_stub();
        stub.script_events([
            list(
                2,
                vec![
                    event("1", 1, "Normal", "Started", 1),
                    event("2", 2, "Normal", "Synced", 1),
                ],
            ),
            list(2, vec![]),
            list(
                4,
                vec![
                    event("1", 4, "Normal", "Started", 2),
                    event("3", 3, "Warning", "SyncFailed", 1),
                ],
            ),
        ]);
        let (result, out) = watched(&stub, &["events", "--watch"], 2);
        result.unwrap();
        assert_eq!(
            out,
            "\
TYPE     REASON    COUNT
Normal   Started   1
Normal   Synced    1
Normal    Started      2
Warning   SyncFailed   1
"
        );
        assert_eq!(stub.event_queries(), ["", "after=2", "after=2"]);
    }

    #[test]
    fn watch_prints_each_object_in_other_formats() {
        let stub = events_stub();
        stub.script_events([
            list(1, vec![event("1", 1, "Normal", "Started", 1)]),
            list(2, vec![event("2", 2, "Normal", "Synced", 1)]),
        ]);
        let (result, out) = watched(&stub, &["ev", "-w", "-o", "name"], 1);
        result.unwrap();
        assert_eq!(out, "event/1\nevent/2\n");
    }

    #[test]
    fn a_failed_poll_does_not_end_the_watch() {
        let stub = events_stub();
        stub.script_events([
            list(1, vec![event("1", 1, "Normal", "Started", 1)]),
            json!(null),
            list(2, vec![event("2", 2, "Warning", "SyncFailed", 1)]),
        ]);
        let (result, out) = watched(&stub, &["events", "--watch", "--no-headers"], 2);
        result.unwrap();
        assert!(out.ends_with("Warning   SyncFailed   1\n"), "{out}");
        assert_eq!(stub.event_queries(), ["", "after=1", "after=1"]);
    }

    #[test]
    fn only_a_watchable_type_is_watched() {
        let stub = events_stub();
        let (result, _) = watched(&stub, &["projects", "--watch"], 0);
        assert_eq!(
            result.unwrap_err().to_string(),
            "projects cannot be watched"
        );
        let (result, _) = watched(&stub, &["events", "projects", "--watch"], 0);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("one resource type")
        );
        let (result, _) = watched(&stub, &["events/1", "--watch"], 0);
        assert!(result.unwrap_err().to_string().contains("not \"events/1\""));
    }
}
