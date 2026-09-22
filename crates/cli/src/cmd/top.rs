//! `hldr top`: what the site served, as `kubectl top` shows what pods use.
//! Visits per day, or the most viewed pages or referring hosts, counted by
//! the server without tracking anyone.

use std::io::Write;

use anyhow::{Result, bail};
use hldr_core::api::{DailyMetrics, PageMetrics, ReferrerMetrics, Since};
use serde_json::Value;

use crate::client::Client;
use crate::color::{self, Painter, Role, Term};
use crate::print;

/// Rows of pages or referrers shown when `--limit` is not given.
const DEFAULT_LIMIT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum View {
    /// Totals for the period and one row per day
    Daily,
    /// Views and visitors per route, most viewed first
    Pages,
    /// Views per referring host, most first
    Referrers,
}

impl View {
    fn path(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Pages => "pages",
            Self::Referrers => "referrers",
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct Args {
    #[arg(value_enum, default_value_t = View::Daily)]
    view: View,
    /// The period: a number of days up to today, such as 7d or 30d, or all
    #[arg(long, default_value = "7d", value_parser = |text: &str| text.parse::<Since>())]
    since: Since,
    /// At most this many rows: the most viewed pages or hosts (20 by
    /// default), or the latest days
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    limit: Option<u16>,
    /// A table, or json or yaml
    #[arg(short, long)]
    output: Option<String>,
}

pub fn run(out: &mut dyn Write, term: &Term, client: &Client, args: &Args) -> Result<bool> {
    let format = match args.output.as_deref() {
        None => None,
        Some(format @ ("json" | "yaml")) => Some(format),
        Some(other) => bail!("unknown output {other:?}: use json or yaml"),
    };
    let path = format!("/api/v1/metrics/{}?since={}", args.view.path(), args.since);
    let Some(mut value) = client.get_optional(&path)? else {
        bail!(
            "{} does not count visits: metrics need hldr-server 4.4.0 or later",
            client.base()
        );
    };
    limit(&mut value, args.view, args.limit);
    let paint = term.out();
    match format {
        Some("yaml") => write!(
            out,
            "{}",
            color::yaml(paint, &serde_saphyr::to_string(&value)?)
        )?,
        Some(_) => writeln!(
            out,
            "{}",
            color::json(paint, &serde_json::to_string_pretty(&value)?)
        )?,
        None => match args.view {
            View::Daily => write!(
                out,
                "{}",
                daily_text(paint, &serde_json::from_value(value)?)?
            )?,
            View::Pages => {
                let pages: PageMetrics = serde_json::from_value(value)?;
                let rows = pages
                    .items
                    .iter()
                    .map(|page| {
                        vec![
                            page.route.clone(),
                            page.views.to_string(),
                            page.visitors.to_string(),
                        ]
                    })
                    .collect();
                ranking(
                    out,
                    term,
                    &pages.period.from,
                    &["ROUTE", "VIEWS", "VISITORS"],
                    rows,
                )?;
            }
            View::Referrers => {
                let referrers: ReferrerMetrics = serde_json::from_value(value)?;
                let rows = referrers
                    .items
                    .iter()
                    .map(|referrer| vec![referrer.host.clone(), referrer.views.to_string()])
                    .collect();
                ranking(out, term, &referrers.period.from, &["HOST", "VIEWS"], rows)?;
            }
        },
    }
    Ok(true)
}

/// Keeps the first `limit` pages or hosts, or the last `limit` days.
fn limit(value: &mut Value, view: View, limit: Option<u16>) {
    let limit = match (view, limit) {
        (_, Some(limit)) => usize::from(limit),
        (View::Daily, None) => return,
        (_, None) => DEFAULT_LIMIT,
    };
    let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) else {
        return;
    };
    match view {
        View::Daily => {
            let older = items.len().saturating_sub(limit);
            items.drain(..older);
        }
        View::Pages | View::Referrers => items.truncate(limit),
    }
}

fn ranking(
    out: &mut dyn Write,
    term: &Term,
    from: &str,
    headers: &[&str],
    rows: Vec<Vec<String>>,
) -> Result<()> {
    if rows.is_empty() {
        term.warning(&format!("nothing counted since {from}"));
        return Ok(());
    }
    let headers = headers.iter().map(|header| (*header).to_owned()).collect();
    print::table(out, term.out(), headers, rows, &[], false)?;
    Ok(())
}

/// The period's totals laid out as `describe` lays out a resource, then a
/// row per day.
fn daily_text(paint: Painter<'_>, metrics: &DailyMetrics) -> Result<String> {
    let label = |name: &str| paint.nth(Role::DescribeKey, 0, name);
    let number = |count: u64| paint.paint(Role::DataNumber, &count.to_string());
    let period = &metrics.period;
    let mut out = format!(
        "{}:     {}\n{}:      {}\n{}:   {}\n{}:       {}\n{}:\n",
        label("Period"),
        paint.paint(
            Role::DataString,
            &format!("{} to {} ({})", period.from, period.to, period.since)
        ),
        label("Views"),
        number(metrics.total.views),
        label("Visitors"),
        number(metrics.total.visitors),
        label("Bots"),
        number(metrics.total.bots),
        label("Days"),
    );
    let rows = metrics
        .items
        .iter()
        .map(|day| {
            vec![
                day.day.clone(),
                day.visits.views.to_string(),
                day.visits.visitors.to_string(),
                day.visits.bots.to_string(),
            ]
        })
        .collect();
    let mut table = Vec::new();
    print::table(
        &mut table,
        paint,
        ["DAY", "VIEWS", "VISITORS", "BOTS"]
            .map(str::to_owned)
            .to_vec(),
        rows,
        &[],
        false,
    )?;
    for line in String::from_utf8_lossy(&table).lines() {
        out.push_str(&format!("  {line}\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::{Path, RawQuery};
    use axum::routing::get;
    use serde_json::json;

    use super::*;
    use crate::stub;

    /// A server that answers every metrics view, and records what it was
    /// asked.
    fn server(asked: Arc<Mutex<Vec<String>>>) -> String {
        let period = json!({"since": "3d", "from": "2026-09-20", "to": "2026-09-22"});
        let app = Router::new().route(
            "/api/v1/metrics/{view}",
            get(move |Path(view): Path<String>, RawQuery(query): RawQuery| {
                asked
                    .lock()
                    .unwrap()
                    .push(format!("{view}?{}", query.unwrap_or_default()));
                let period = period.clone();
                async move {
                    axum::Json(match view.as_str() {
                        "daily" => json!({
                            "kind": "DailyMetrics", "period": period,
                            "total": {"views": 12, "visitors": 5, "bots": 30},
                            "items": [
                                {"day": "2026-09-20", "views": 0, "visitors": 0, "bots": 2},
                                {"day": "2026-09-21", "views": 10, "visitors": 4, "bots": 20},
                                {"day": "2026-09-22", "views": 2, "visitors": 1, "bots": 8},
                            ],
                        }),
                        "pages" => json!({
                            "kind": "PageMetrics", "period": period,
                            "items": [
                                {"route": "/", "views": 8, "visitors": 4},
                                {"route": "/projects/atlas", "views": 3, "visitors": 2},
                                {"route": "404", "views": 1, "visitors": 1},
                            ],
                        }),
                        _ => json!({"kind": "ReferrerMetrics", "period": period, "items": []}),
                    })
                }
            }),
        );
        stub::spawn(app)
    }

    fn top(base: &str, argv: &[&str]) -> Result<String> {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        let mut full = vec!["top"];
        full.extend(argv);
        let args = Cli::try_parse_from(full)?.args;
        let mut out = Vec::new();
        run(
            &mut out,
            &Term::plain(),
            &Client::new(base.to_owned()),
            &args,
        )?;
        Ok(String::from_utf8(out)?)
    }

    #[test]
    fn daily_shows_the_totals_then_each_day() {
        let asked = Arc::default();
        let base = server(Arc::clone(&asked));
        assert_eq!(
            top(&base, &[]).unwrap(),
            "\
Period:     2026-09-20 to 2026-09-22 (3d)
Views:      12
Visitors:   5
Bots:       30
Days:
  DAY          VIEWS   VISITORS   BOTS
  2026-09-20   0       0          2
  2026-09-21   10      4          20
  2026-09-22   2       1          8
"
        );
        let out = top(&base, &["daily", "--limit", "1", "--since", "all"]).unwrap();
        assert!(
            out.ends_with("BOTS\n  2026-09-22   2       1          8\n"),
            "{out}"
        );
        assert_eq!(
            *asked.lock().unwrap(),
            ["daily?since=7d", "daily?since=all"]
        );
    }

    #[test]
    fn pages_and_referrers_rank() {
        let asked = Arc::default();
        let base = server(Arc::clone(&asked));
        assert_eq!(
            top(&base, &["pages", "--since", "30d", "--limit", "2"]).unwrap(),
            "\
ROUTE             VIEWS   VISITORS
/                 8       4
/projects/atlas   3       2
"
        );
        assert_eq!(top(&base, &["referrers"]).unwrap(), "");
        let json: Value =
            serde_json::from_str(&top(&base, &["pages", "--limit", "1", "-o", "json"]).unwrap())
                .unwrap();
        assert_eq!(json["kind"], "PageMetrics");
        assert_eq!(json["items"].as_array().unwrap().len(), 1);
        let yaml = top(&base, &["-o", "yaml"]).unwrap();
        assert!(yaml.contains("kind: DailyMetrics"), "{yaml}");
        assert_eq!(asked.lock().unwrap()[0], "pages?since=30d");
    }

    #[test]
    fn rejects_what_it_cannot_ask_or_show() {
        let base = server(Arc::default());
        for argv in [
            &["--since", "1y"][..],
            &["--since", "0d"],
            &["--limit", "0"],
            &["visitors"],
        ] {
            assert!(top(&base, argv).is_err(), "{argv:?}");
        }
        assert!(top(&base, &["-o", "wide"]).is_err());

        let old = stub::spawn(Router::new());
        let err = top(&old, &[]).unwrap_err();
        assert!(err.to_string().contains("4.4.0 or later"), "{err}");
    }
}
