//! The API policy, checked: within a major, `/api/v1` and the manifests only
//! grow. Every type the API serves or reads is rendered as JSON Schema and
//! compared with `api-snapshot.json`:
//!
//! - a removed field, a changed type, a field that became required or
//!   optional, a new required field, a removed enum value or variant: the
//!   test fails with the policy, since a client within the supported skew
//!   may depend on the old shape;
//! - anything else that changed (an added optional field, a new variant):
//!   the test fails until the snapshot is regenerated, so every change to
//!   the contract shows in the diff of the commit that made it.
//!
//! Regenerate with `UPDATE_API_SNAPSHOT=1 cargo test -p hldr-core --test
//! api_contract`. A breaking change regenerates only with
//! `UPDATE_API_SNAPSHOT=breaking`, and must ship as a breaking release
//! (`feat!`) or under a new API version served beside v1.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hldr_core::api::{
    self, ApiResource, ApiVersions, DailyMetrics, EventList, List, PageMetrics, Problem,
    ReferrerMetrics, SyncRequest, SyncStatus,
};
use schemars::{JsonSchema, schema_for};
use serde_json::{Map, Value, json};

/// Keywords that document a schema without constraining it.
const ANNOTATIONS: &[&str] = &[
    "description",
    "title",
    "default",
    "examples",
    "$comment",
    "$schema",
    "deprecated",
    "readOnly",
    "writeOnly",
];

/// Keywords the checker compares on its own terms; any other keyword must
/// stay equal.
const STRUCTURAL: &[&str] = &[
    "$ref",
    "$defs",
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "oneOf",
    "anyOf",
    "allOf",
];

const POLICY: &str = "the API only grows between breaking releases: a breaking change ships \
                      as one (`feat!`) or under a new API version, and regenerates with \
                      UPDATE_API_SNAPSHOT=breaking";

#[test]
fn the_api_only_grows() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("api-snapshot.json");
    let current = surface();
    let snapshot: Option<Value> = std::fs::read_to_string(&path).ok().map(|text| {
        serde_json::from_str(&text).expect("api-snapshot.json is not JSON; regenerate it")
    });
    let problems = snapshot
        .as_ref()
        .map(|old| breaking(old, &current))
        .unwrap_or_default();
    let update = std::env::var("UPDATE_API_SNAPSHOT").ok();

    if update.as_deref() != Some("breaking") {
        assert!(
            problems.is_empty(),
            "{POLICY}:\n  {}",
            problems.join("\n  ")
        );
    }
    if update.is_some() {
        let text = serde_json::to_string_pretty(&current).expect("a schema serializes");
        std::fs::write(&path, text + "\n").expect("write api-snapshot.json");
        return;
    }
    let why = if snapshot.is_none() {
        "api-snapshot.json is missing"
    } else {
        "the API changed without breaking it"
    };
    assert!(
        snapshot.as_ref() == Some(&current),
        "{why}; regenerate it with UPDATE_API_SNAPSHOT=1 cargo test -p hldr-core --test \
         api_contract, and commit it"
    );
}

/// Every schema the API serves or reads, without annotations, keys sorted.
fn surface() -> Value {
    let kinds = Value::Object(api::schemas(&[]));
    let mut wire = Map::new();
    let mut add = |name: &str, schema: Value| {
        wire.insert(name.to_owned(), schema);
    };
    add("APIVersions", schema::<ApiVersions>());
    add("APIResourceList", schema::<List<ApiResource>>());
    add("SyncStatus", schema::<SyncStatus>());
    add("SyncRequest", schema::<SyncRequest>());
    add("EventList", schema::<EventList>());
    add("DailyMetrics", schema::<DailyMetrics>());
    add("PageMetrics", schema::<PageMetrics>());
    add("ReferrerMetrics", schema::<ReferrerMetrics>());
    add("Health", schema::<hldr_core::Health>());
    add("Problem", schema::<Problem>());
    canonical(&json!({"kinds": kinds, "wire": wire}))
}

fn schema<T: JsonSchema>() -> Value {
    schema_for!(T).to_value()
}

/// Drops annotations and sorts keys, so the snapshot is the same whatever
/// map order serde_json was built with. A schema's annotations go; the keys
/// of a map of names (`properties`, `$defs`) are names, even `title`.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(sorted(map.iter().filter_map(|(key, value)| {
            if ANNOTATIONS.contains(&key.as_str()) {
                return None;
            }
            let value = match (key.as_str(), value) {
                ("properties" | "$defs", Value::Object(named)) => Value::Object(sorted(
                    named.iter().map(|(name, schema)| (name, canonical(schema))),
                )),
                _ => canonical(value),
            };
            Some((key, value))
        }))),
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn sorted<'k>(entries: impl Iterator<Item = (&'k String, Value)>) -> Map<String, Value> {
    let entries: BTreeMap<&String, Value> = entries.collect();
    entries.into_iter().map(|(k, v)| (k.clone(), v)).collect()
}

/// What in `new` breaks a reader or writer of `old`; each top-level schema
/// is checked against its own `$defs`.
fn breaking(old: &Value, new: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    for section in ["kinds", "wire"] {
        let (Some(old), Some(new)) = (old[section].as_object(), new[section].as_object()) else {
            continue;
        };
        for (name, old_schema) in old {
            match new.get(name) {
                None => problems.push(format!("{section}.{name}: removed")),
                Some(new_schema) => {
                    let mut checker = Checker {
                        old_root: old_schema,
                        new_root: new_schema,
                        seen: BTreeSet::new(),
                        problems: &mut problems,
                    };
                    checker.check(old_schema, new_schema, name);
                }
            }
        }
    }
    problems
}

struct Checker<'a> {
    old_root: &'a Value,
    new_root: &'a Value,
    /// Pairs of definitions already compared, so recursive types end.
    seen: BTreeSet<(String, String)>,
    problems: &'a mut Vec<String>,
}

impl<'a> Checker<'a> {
    fn check(&mut self, old: &'a Value, new: &'a Value, at: &str) {
        if let (Some(old_ref), Some(new_ref)) = (reference(old), reference(new))
            && !self.seen.insert((old_ref.to_owned(), new_ref.to_owned()))
        {
            return;
        }
        let old = resolve(self.old_root, old);
        let new = resolve(self.new_root, new);

        let (old_types, new_types) = (types(old), types(new));
        if old_types != new_types {
            self.problem(
                at,
                format!("type changed from {old_types:?} to {new_types:?}"),
            );
        }
        self.properties(old, new, at);
        match (&old["additionalProperties"], &new["additionalProperties"]) {
            (old_extra @ Value::Object(_), new_extra @ Value::Object(_)) => {
                self.check(old_extra, new_extra, &format!("{at}.*"));
            }
            (old_extra, Value::Bool(false)) if old_extra != &Value::Bool(false) => {
                self.problem(
                    at,
                    "no longer accepts fields it does not declare".to_owned(),
                );
            }
            _ => {}
        }
        if let (Some(_), Some(_)) = (old.get("items"), new.get("items")) {
            self.check(&old["items"], &new["items"], &format!("{at}[]"));
        }
        if let Some(values) = old["enum"].as_array() {
            let kept = new["enum"].as_array();
            for value in values {
                if !kept.is_some_and(|kept| kept.contains(value)) {
                    self.problem(at, format!("value {value} removed"));
                }
            }
        }
        for keyword in ["oneOf", "anyOf", "allOf"] {
            self.variants(old, new, keyword, at);
        }
        let others = |schema: &Value| -> BTreeMap<String, Value> {
            schema
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, _)| !STRUCTURAL.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        };
        let (old_rest, new_rest) = (others(old), others(new));
        for (key, value) in &old_rest {
            if new_rest.get(key) != Some(value) {
                self.problem(at, format!("{key} changed"));
            }
        }
        for key in new_rest.keys().filter(|key| !old_rest.contains_key(*key)) {
            self.problem(at, format!("{key} added, which constrains what was valid"));
        }
    }

    fn properties(&mut self, old: &'a Value, new: &'a Value, at: &str) {
        let Some(old_props) = old["properties"].as_object() else {
            return;
        };
        let new_props = &new["properties"];
        let required = |schema: &Value| -> BTreeSet<String> {
            schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect()
        };
        let (old_required, new_required) = (required(old), required(new));
        for (name, old_prop) in old_props {
            let field = format!("{at}.{name}");
            let Some(new_prop) = new_props.get(name) else {
                self.problems.push(format!("{field}: removed"));
                continue;
            };
            match (old_required.contains(name), new_required.contains(name)) {
                (false, true) => self.problems.push(format!("{field}: became required")),
                (true, false) => self.problems.push(format!("{field}: became optional")),
                _ => {}
            }
            self.check(old_prop, new_prop, &field);
        }
        let added = new_props.as_object().into_iter().flat_map(Map::keys);
        for name in added.filter(|name| !old_props.contains_key(*name)) {
            if new_required.contains(name) {
                self.problems.push(format!(
                    "{at}.{name}: a new field must be optional, since the other side of the skew \
                     lacks it"
                ));
            }
        }
    }

    /// Each old variant must survive: matched by its constant, its
    /// definition, or else its type.
    fn variants(&mut self, old: &'a Value, new: &'a Value, keyword: &str, at: &str) {
        let Some(old_variants) = old[keyword].as_array() else {
            return;
        };
        let new_variants: &'a [Value] = new[keyword].as_array().map_or(&[], Vec::as_slice);
        for variant in old_variants {
            let found = new_variants.iter().find(|candidate| {
                match (variant.get("const"), reference(variant)) {
                    (Some(value), _) => candidate.get("const") == Some(value),
                    (None, Some(name)) => reference(candidate) == Some(name),
                    (None, None) => {
                        candidate.get("const").is_none()
                            && reference(candidate).is_none()
                            && types(candidate) == types(variant)
                    }
                }
            });
            match found {
                Some(candidate) => self.check(variant, candidate, at),
                None => self.problem(at, format!("{keyword} variant {variant} removed")),
            }
        }
    }

    fn problem(&mut self, at: &str, what: String) {
        self.problems.push(format!("{at}: {what}"));
    }
}

fn reference(schema: &Value) -> Option<&str> {
    schema["$ref"].as_str()?.strip_prefix("#/$defs/")
}

fn resolve<'v>(root: &'v Value, schema: &'v Value) -> &'v Value {
    match reference(schema) {
        Some(name) => root["$defs"].get(name).unwrap_or(&Value::Null),
        None => schema,
    }
}

fn types(schema: &Value) -> BTreeSet<String> {
    match &schema["type"] {
        Value::String(one) => BTreeSet::from([one.clone()]),
        Value::Array(many) => many
            .iter()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect(),
        _ => BTreeSet::new(),
    }
}

mod checker {
    use super::*;

    fn wire(schema: Value) -> Value {
        json!({"kinds": {}, "wire": {"T": schema}})
    }

    fn object(properties: Value, required: &[&str]) -> Value {
        json!({"type": "object", "properties": properties, "required": required})
    }

    fn problems(old: Value, new: Value) -> Vec<String> {
        breaking(&wire(old), &wire(new))
    }

    #[test]
    fn an_added_optional_field_is_not_breaking() {
        let old = object(json!({"a": {"type": "string"}}), &["a"]);
        let new = object(
            json!({"a": {"type": "string"}, "b": {"type": ["string", "null"]}}),
            &["a"],
        );
        assert_eq!(problems(old, new), Vec::<String>::new());
    }

    #[test]
    fn removing_retyping_and_requiring_are() {
        let old = object(
            json!({"a": {"type": "string"}, "b": {"type": "string"}, "c": {"type": "integer"}, "d": {"type": "string"}}),
            &["a", "d"],
        );
        let new = object(
            json!({"a": {"type": "string"}, "c": {"type": "string"}, "d": {"type": "string"}, "e": {"type": "string"}}),
            &["a", "e"],
        );
        let found = problems(old, new);
        for expected in [
            "T.b: removed",
            "T.c: type changed",
            "T.d: became optional",
            "T.e: a new field must be optional",
        ] {
            assert!(
                found.iter().any(|p| p.starts_with(expected)),
                "{expected} in {found:?}"
            );
        }
        assert_eq!(found.len(), 4, "{found:?}");
    }

    #[test]
    fn looks_through_definitions() {
        let old = json!({
            "$ref": "#/$defs/Inner",
            "$defs": {"Inner": object(json!({"x": {"type": "string"}}), &["x"])},
        });
        let new = json!({
            "$ref": "#/$defs/Inner",
            "$defs": {"Inner": object(json!({}), &[])},
        });
        assert_eq!(problems(old, new), ["T.x: removed"]);
    }

    #[test]
    fn variants_and_enum_values_may_grow_but_not_shrink() {
        let old = json!({"oneOf": [{"const": "a"}, {"const": "b"}]});
        let grown = json!({"oneOf": [{"const": "a"}, {"const": "b"}, {"const": "c"}]});
        assert!(problems(old.clone(), grown).is_empty());
        let shrunk = json!({"oneOf": [{"const": "a"}]});
        assert_eq!(problems(old, shrunk).len(), 1);

        let old = json!({"type": "string", "enum": ["x", "y"]});
        let shrunk = json!({"type": "string", "enum": ["x"]});
        assert_eq!(problems(old, shrunk), [r#"T: value "y" removed"#]);
    }

    #[test]
    fn closing_an_object_to_unknown_fields_is_breaking() {
        let old = json!({"type": "object", "properties": {}});
        let closed = json!({"type": "object", "properties": {}, "additionalProperties": false});
        assert_eq!(problems(old, closed).len(), 1);
    }

    #[test]
    fn fields_named_like_annotations_stay() {
        let schema = json!({
            "title": "T",
            "properties": {"title": {"type": "string", "description": "d"}},
            "$defs": {"default": {"type": "string"}},
        });
        assert_eq!(
            canonical(&schema),
            json!({
                "properties": {"title": {"type": "string"}},
                "$defs": {"default": {"type": "string"}},
            })
        );
    }

    #[test]
    fn a_removed_schema_is_breaking() {
        let old = wire(json!({"type": "string"}));
        let new = json!({"kinds": {}, "wire": {}});
        assert_eq!(breaking(&old, &new), ["wire.T: removed"]);
    }
}
