//! Profile → patch-layer composition over TOML rows.
//!
//! Port of the `app-boot` composition mechanics (profile → bundles →
//! user patch → CLI overlays) onto AETHER-native TOML:
//!
//! ```toml
//! [[plugin]]
//! id = "tools-fs"                 # stable patch target
//! plugin = "builtin:tools-fs"     # factory name in the registry
//! inject = []                     # extra service gates (optional)
//! disabled = false                # (optional)
//! only_os = "windows"             # (optional)
//! except_os = "macos"             # (optional)
//! requires_env = ["FOO"]          # (optional)
//!
//! [plugin.config]                 # (optional, whole-row replaced)
//! max_entries = 200
//! ```
//!
//! Layer order: bundled base → user file (`~/.aether/plugins.toml`)
//! → `--plugin-patch` overlays in order. A row whose `id` matches an
//! earlier row **replaces the whole row** (config is NOT merged —
//! this eliminates merge-order bugs by construction). A brand-new id
//! appends (recorded as an insert warning so spanning overlays stay
//! audible). An unknown plugin factory name fails boot loudly.
//!
//! Skip conditions (recorded with reasons, never silent):
//! `disabled`, `only_os`/`except_os` mismatch, missing
//! `requires_env` entries.
//!
//! [`dump`] renders the effective composition back to TOML with
//! `# origin:` provenance comments — the `--dump-plugins` surface.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// One composition row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Stable id: patches address rows by this.
    pub id: String,
    /// Factory name in the [`PluginRegistry`].
    pub plugin: String,
    /// Service keys that must exist before activation.
    /// Defaults to the plugin's own `inject()`.
    #[serde(default)]
    pub inject: Vec<String>,
    /// Skip this row (recorded with reason).
    #[serde(default)]
    pub disabled: bool,
    /// Only activate on this OS (`std::env::consts::OS`).
    #[serde(default)]
    pub only_os: Option<String>,
    /// Skip activation on this OS.
    #[serde(default)]
    pub except_os: Option<String>,
    /// Skip activation unless all these env vars are set.
    #[serde(default)]
    pub requires_env: Vec<String>,
    /// Whole-row config (replaced, never merged).
    #[serde(default)]
    pub config: Value,
}

impl Row {
    pub fn new(id: impl Into<String>, plugin: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            plugin: plugin.into(),
            inject: Vec::new(),
            disabled: false,
            only_os: None,
            except_os: None,
            requires_env: Vec::new(),
            config: Value::Object(Default::default()),
        }
    }

    /// Skip reason, if this row must not activate in this process.
    pub fn skip_reason(&self) -> Option<String> {
        if self.disabled {
            return Some("disabled".to_string());
        }
        const OS: &str = std::env::consts::OS;
        if let Some(only) = &self.only_os {
            if only != OS {
                return Some(format!("only_os={only} (this is {OS})"));
            }
        }
        if let Some(except) = &self.except_os {
            if except == OS {
                return Some(format!("except_os={except} (this is {OS})"));
            }
        }
        let missing: Vec<&str> = self
            .requires_env
            .iter()
            .filter(|k| std::env::var_os(k).is_none())
            .map(|k| k.as_str())
            .collect();
        if !missing.is_empty() {
            return Some(format!("missing env: {}", missing.join(", ")));
        }
        None
    }
}

/// Where a composed row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Bundled base profile.
    Base,
    /// User file (`~/.aether/plugins.toml`).
    UserFile,
    /// `--plugin-patch` overlay, by position.
    Overlay(u16),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Base => write!(f, "base"),
            Origin::UserFile => write!(f, "user-file"),
            Origin::Overlay(i) => write!(f, "overlay:{i}"),
        }
    }
}

/// A row with its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedRow {
    pub row: Row,
    pub origin: Origin,
    /// True when this row replaced an earlier row with the same id.
    pub replaced: bool,
}

/// The effective composition plus non-fatal notes.
#[derive(Debug, Clone)]
pub struct Composition {
    pub rows: Vec<ComposedRow>,
    /// Non-fatal notes (inserted-new-id, skipped rows with reasons).
    pub warnings: Vec<String>,
}

/// Named plugin factories: `Fn() -> Arc<dyn Plugin>`.
pub type Factory = Box<dyn Fn() -> std::sync::Arc<dyn crate::plugin::Plugin> + Send + Sync>;

/// Factory registry. Populated by the embedding binary (CLI):
/// built-ins always, third-party rows resolve here too (P3).
#[derive(Default)]
pub struct PluginRegistry {
    factories: HashMap<String, Factory>,
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.factories.keys().map(|s| s.as_str()).collect();
        names.sort();
        f.debug_struct("PluginRegistry").field("factories", &names).finish()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, factory: Factory) {
        self.factories.insert(name.into(), factory);
    }

    pub fn create(&self, name: &str) -> Option<std::sync::Arc<dyn crate::plugin::Plugin>> {
        self.factories.get(name).map(|f| f())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.factories.contains_key(name)
    }
}

/// Compose layers into an effective row list.
///
/// `base` is the bundled profile; `layers` are `(origin, rows)` in
/// application order (user file, then overlays). Same id ⇒ whole-row
/// replace (marked `replaced`, new origin). New id ⇒ append + insert
/// warning. Rows are NOT filtered here (skip evaluation happens at
/// boot so dumps show intent); use [`ComposedRow`] + [`Row::skip_reason`].
pub fn compose(base: Vec<Row>, layers: Vec<(Origin, Vec<Row>)>) -> Composition {
    let mut rows: Vec<ComposedRow> = base
        .into_iter()
        .map(|row| ComposedRow { row, origin: Origin::Base, replaced: false })
        .collect();
    let mut warnings = Vec::new();
    for (origin, layer) in layers {
        for row in layer {
            match rows.iter_mut().find(|r| r.row.id == row.id) {
                Some(existing) => {
                    existing.row = row;
                    existing.origin = origin;
                    existing.replaced = true;
                }
                None => {
                    warnings.push(format!(
                        "overlay {origin}: unknown id '{}' inserted as a new row",
                        row.id
                    ));
                    rows.push(ComposedRow { row, origin, replaced: false });
                }
            }
        }
    }
    Composition { rows, warnings }
}

/// Parse a TOML composition file (`[[plugin]]` rows).
pub fn parse_file(text: &str) -> Result<Vec<Row>, CompositionError> {
    let raw: TomlFile = toml::from_str(text)?;
    Ok(raw.plugin.into_iter().map(shim_row).collect::<Vec<_>>())
}

#[derive(Debug, thiserror::Error)]
pub enum CompositionError {
    #[error("cannot parse composition TOML: {0}")]
    Parse(#[from] toml::de::Error),
}

// --- TOML/JSON bridging -----------------------------------------------
// `Row.config` is `serde_json::Value` so plugins stay format-agnostic.
// TOML tables deserialize into an intermediate `toml::Value`, then
// convert losslessly (datetime → RFC3339 string; TOML has no null).

#[derive(Deserialize)]
struct TomlFile {
    #[serde(default)]
    plugin: Vec<TomlRow>,
}

#[derive(Deserialize)]
struct TomlRow {
    id: String,
    plugin: String,
    #[serde(default)]
    inject: Vec<String>,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    only_os: Option<String>,
    #[serde(default)]
    except_os: Option<String>,
    #[serde(default)]
    requires_env: Vec<String>,
    #[serde(default = "default_toml_table")]
    config: toml::Value,
}

fn default_toml_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => Value::from(f),
        toml::Value::Boolean(b) => Value::from(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect())
        }
    }
}

fn shim_row(r: TomlRow) -> Row {
    Row {
        id: r.id,
        plugin: r.plugin,
        inject: r.inject,
        disabled: r.disabled,
        only_os: r.only_os,
        except_os: r.except_os,
        requires_env: r.requires_env,
        config: toml_to_json(r.config),
    }
}

/// Render the effective composition back to TOML with `# origin:`
/// provenance comments (the `--dump-plugins` surface). Skipped rows
/// are included with a `# skipped: <reason>` comment so dumps show
/// intent, not just outcome.
pub fn dump(composition: &Composition) -> String {
    let mut out = String::from("# Effective AETHER plugin composition.\n");
    out.push_str("# Same id in a later layer replaces the whole row (config is NOT merged).\n");
    for row in &composition.rows {
        out.push_str(&format!(
            "\n# origin: {}{}\n",
            row.origin,
            if row.replaced { " (replaced earlier row)" } else { "" }
        ));
        if let Some(reason) = row.row.skip_reason() {
            out.push_str(&format!("# skipped: {reason}\n"));
        }
        out.push_str("[[plugin]]\n");
        out.push_str(&format!("id = {:?}\n", row.row.id));
        out.push_str(&format!("plugin = {:?}\n", row.row.plugin));
        if !row.row.inject.is_empty() {
            out.push_str(&format!("inject = {:?}\n", row.row.inject));
        }
        if row.row.disabled {
            out.push_str("disabled = true\n");
        }
        if let Some(os) = &row.row.only_os {
            out.push_str(&format!("only_os = {os:?}\n"));
        }
        if let Some(os) = &row.row.except_os {
            out.push_str(&format!("except_os = {os:?}\n"));
        }
        if !row.row.requires_env.is_empty() {
            out.push_str(&format!("requires_env = {:?}\n", row.row.requires_env));
        }
        if row.row.config != Value::Object(Default::default()) {
            out.push_str("[plugin.config]\n");
            dump_json_value(&mut out, &row.row.config, 0);
        }
    }
    if !composition.warnings.is_empty() {
        out.push_str("\n# warnings:\n");
        for w in &composition.warnings {
            out.push_str(&format!("# - {w}\n"));
        }
    }
    out
}

fn dump_json_value(out: &mut String, v: &Value, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Value::Object(map) = v {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            let val = &map[k];
            match val {
                Value::Object(_) => {
                    out.push_str(&format!("[plugin.config.{k}]\n"));
                    dump_json_value(out, val, depth + 1);
                }
                _ => {
                    out.push_str(&format!("{k} = {}\n", json_to_toml_literal(val)));
                }
            }
        }
    }
}

fn json_to_toml_literal(v: &Value) -> String {
    match v {
        Value::Null => "\"null\"".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(json_to_toml_literal).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(_) => "{ ... }".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> Row {
        Row::new(id, format!("builtin:{id}"))
    }

    #[test]
    fn whole_row_replace_never_merges_config() {
        let mut base = row("tools-fs");
        base.config = serde_json::json!({"a": 1, "b": 2});
        let mut patch = row("tools-fs");
        patch.config = serde_json::json!({"b": 3});
        let c = compose(vec![base], vec![(Origin::UserFile, vec![patch])]);
        assert_eq!(c.rows.len(), 1);
        assert!(c.rows[0].replaced);
        assert_eq!(c.rows[0].origin, Origin::UserFile);
        // `a` is GONE: whole-row replace, not a merge.
        assert_eq!(c.rows[0].row.config, serde_json::json!({"b": 3}));
    }

    #[test]
    fn unknown_id_appends_with_warning() {
        let c = compose(vec![row("a")], vec![(Origin::Overlay(0), vec![row("zzz")])]);
        assert_eq!(c.rows.len(), 2);
        assert_eq!(c.rows[1].origin, Origin::Overlay(0));
        assert!(!c.rows[1].replaced);
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].contains("zzz"));
    }

    #[test]
    fn skip_reasons_cover_disabled_os_and_env() {
        let mut r = row("x");
        r.disabled = true;
        assert_eq!(r.skip_reason().as_deref(), Some("disabled"));
        let mut r = row("x");
        r.only_os = Some("definitely-not-this-os".into());
        assert!(r.skip_reason().unwrap().contains("only_os"));
        let mut r = row("x");
        r.except_os = Some(std::env::consts::OS.into());
        assert!(r.skip_reason().unwrap().contains("except_os"));
        let mut r = row("x");
        r.requires_env = vec!["AETHER_TEST_MISSING_ENV_VAR_XYZ".into()];
        assert!(r.skip_reason().unwrap().contains("missing env"));
        assert!(row("plain").skip_reason().is_none());
    }

    #[test]
    fn parse_and_dump_roundtrip() {
        let text = r#"
[[plugin]]
id = "tools-fs"
plugin = "builtin:tools-fs"

[[plugin]]
id = "tools-terminal"
plugin = "builtin:tools-terminal"
disabled = true

[plugin.config]
timeout_ms = 60000
"#;
        let rows = parse_file(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].config["timeout_ms"], serde_json::json!(60000));
        let c = compose(rows, vec![]);
        let dumped = dump(&c);
        assert!(dumped.contains("# origin: base"));
        assert!(dumped.contains("# skipped: disabled"));
        // Dumped output parses again with identical row ids.
        let reparsed = parse_file(&dumped).unwrap();
        let ids: Vec<&str> = reparsed.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["tools-fs", "tools-terminal"]);
    }

    #[test]
    fn empty_layers_keep_base_untouched() {
        let c = compose(vec![row("a"), row("b")], vec![]);
        assert_eq!(c.rows.len(), 2);
        assert!(c.warnings.is_empty());
        assert!(c.rows.iter().all(|r| !r.replaced && r.origin == Origin::Base));
    }
}
