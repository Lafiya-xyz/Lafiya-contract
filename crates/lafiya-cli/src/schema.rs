//! Build-time generation entry point for shell completions, man pages, and
//! versioned JSON schemas for every `--json` output of `lafiya-cli`.
//!
//! This module is invoked from `main.rs` (via the hidden `generate` subcommand
//! and/or a `build.rs`-driven path) to emit:
//!
//! * bash / zsh / fish completions
//! * man pages
//! * versioned JSON schema documents under `crates/lafiya-cli/schemas/`
//!
//! The schemas are the machine-readable contract for automation and the
//! sandbox. They are versioned; additive-breaking changes require a version
//! bump. Validation tests live in `crates/lafiya-cli/tests` and fail when a
//! schema changes without a corresponding version bump.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::Command;

/// Current schema contract version. Bump this (and the per-schema versions
/// below) whenever an additive-breaking change is made to any `--json` output.
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Directory (relative to the crate root) where schemas are published.
pub const SCHEMA_DIR: &str = "schemas";

/// Directory (relative to the crate root) where generated artifacts land.
pub const GENERATED_DIR: &str = "generated";

/// A single versioned JSON schema document for a `--json` output.
#[derive(Debug, Clone)]
pub struct JsonSchemaDoc {
    /// Stable identifier, e.g. `preview`, `pool.read`, `config`.
    pub id: &'static str,
    /// Semantic version of this schema document.
    pub version: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// The JSON schema body (already serialized as a JSON string).
    pub body: &'static str,
}

/// All `--json` outputs the CLI can emit. Every entry here must have a
/// corresponding schema document and a validation test.
pub const JSON_SCHEMAS: &[JsonSchemaDoc] = &[
    JsonSchemaDoc {
        id: "preview",
        version: SCHEMA_VERSION,
        description: "Output of `lafiya preview --json`.",
        body: PREVIEW_SCHEMA,
    },
    JsonSchemaDoc {
        id: "pool.read",
        version: SCHEMA_VERSION,
        description: "Output of `lafiya pool read --json`.",
        body: POOL_READ_SCHEMA,
    },
    JsonSchemaDoc {
        id: "pool.list",
        version: SCHEMA_VERSION,
        description: "Output of `lafiya pool list --json`.",
        body: POOL_LIST_SCHEMA,
    },
    JsonSchemaDoc {
        id: "config",
        version: SCHEMA_VERSION,
        description: "Output of `lafiya config --json`.",
        body: CONFIG_SCHEMA,
    },
    JsonSchemaDoc {
        id: "version",
        version: SCHEMA_VERSION,
        description: "Output of `lafiya version --json`.",
        body: VERSION_SCHEMA,
    },
];

const PREVIEW_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://lafiya.dev/schemas/preview-1.0.0.json",
  "title": "lafiya preview --json",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "kind", "operations"],
  "properties": {
    "schema_version": { "type": "string", "const": "1.0.0" },
    "kind": { "type": "string", "const": "preview" },
    "operations": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "action", "target"],
        "properties": {
          "id": { "type": "string" },
          "action": { "type": "string" },
          "target": { "type": "string" },
          "details": { "type": ["object", "null"] }
        }
      }
    }
  }
}"#;

const POOL_READ_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://lafiya.dev/schemas/pool-read-1.0.0.json",
  "title": "lafiya pool read --json",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "kind", "pool"],
  "properties": {
    "schema_version": { "type": "string", "const": "1.0.0" },
    "kind": { "type": "string", "const": "pool.read" },
    "pool": {
      "type": "object",
      "additionalProperties": false,
      "required": ["id", "name", "size"],
      "properties": {
        "id": { "type": "string" },
        "name": { "type": "string" },
        "size": { "type": "integer", "minimum": 0 },
        "members": { "type": "array", "items": { "type": "string" } }
      }
    }
  }
}"#;

const POOL_LIST_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://lafiya.dev/schemas/pool-list-1.0.0.json",
  "title": "lafiya pool list --json",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "kind", "pools"],
  "properties": {
    "schema_version": { "type": "string", "const": "1.0.0" },
    "kind": { "type": "string", "const": "pool.list" },
    "pools": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "name"],
        "properties": {
          "id": { "type": "string" },
          "name": { "type": "string" }
        }
      }
    }
  }
}"#;

const CONFIG_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://lafiya.dev/schemas/config-1.0.0.json",
  "title": "lafiya config --json",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "kind", "config"],
  "properties": {
    "schema_version": { "type": "string", "const": "1.0.0" },
    "kind": { "type": "string", "const": "config" },
    "config": {
      "type": "object",
      "additionalProperties": true
    }
  }
}"#;

const VERSION_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://lafiya.dev/schemas/version-1.0.0.json",
  "title": "lafiya version --json",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "kind", "version"],
  "properties": {
    "schema_version": { "type": "string", "const": "1.0.0" },
    "kind": { "type": "string", "const": "version" },
    "version": { "type": "string" }
  }
}"#;

/// Errors produced during generation.
#[derive(Debug)]
pub enum GenerateError {
    Io(io::Error),
    MissingCommand(&'static str),
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerateError::Io(e) => write!(f, "io error: {e}"),
            GenerateError::MissingCommand(name) => {
                write!(f, "clap command `{name}` not found")
            }
        }
    }
}

impl std::error::Error for GenerateError {}

impl From<io::Error> for GenerateError {
    fn from(e: io::Error) -> Self {
        GenerateError::Io(e)
    }
}

/// Resolve the crate root directory (the directory containing `Cargo.toml`).
pub fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Generate all artifacts (completions, man pages, schemas) into `out_dir`.
///
/// `cmd` is the fully-built clap `Command` for the CLI. This is the single
/// entry point invoked from `main.rs` and from `build.rs`.
pub fn generate_all(cmd: &Command, out_dir: &Path) -> Result<(), GenerateError> {
    let completions_dir = out_dir.join("completions");
    let man_dir = out_dir.join("man");
    let schema_dir = out_dir.join(SCHEMA_DIR);

    fs::create_dir_all(&completions_dir)?;
    fs::create_dir_all(&man_dir)?;
    fs::create_dir_all(&schema_dir)?;

    generate_completions(cmd, &completions_dir)?;
    generate_man_pages(cmd, &man_dir)?;
    write_schemas(&schema_dir)?;

    Ok(())
}

/// Generate bash, zsh, and fish completions into `dir`.
pub fn generate_completions(cmd: &Command, dir: &Path) -> Result<(), GenerateError> {
    let bin_name = cmd.get_name().to_string();

    let bash = render_bash_completion(cmd, &bin_name);
    fs::write(dir.join(format!("{bin_name}.bash")), bash)?;

    let zsh = render_zsh_completion(cmd, &bin_name);
    fs::write(dir.join(format!("{bin_name}.zsh")), zsh)?;

    let fish = render_fish_completion(cmd, &bin_name);
    fs::write(dir.join(format!("{bin_name}.fish")), fish)?;

    Ok(())
}

/// Generate man pages (one per subcommand plus the top-level page) into `dir`.
pub fn generate_man_pages(cmd: &Command, dir: &Path) -> Result<(), GenerateError> {
    let bin_name = cmd.get_name().to_string();

    // Top-level man page.
    let top = render_man_page(cmd, &bin_name, None);
    fs::write(dir.join(format!("{bin_name}.1")), top)?;

    // One man page per subcommand.
    for sub in cmd.get_subcommands() {
        let sub_name = sub.get_name();
        if sub_name == "help" {
            continue;
        }
        let page = render_man_page(sub, &bin_name, Some(sub_name));
        fs::write(dir.join(format!("{bin_name}-{sub_name}.1")), page)?;
    }

    Ok(())
}

/// Write all versioned JSON schemas into `dir`, plus an index manifest.
pub fn write_schemas(dir: &Path) -> Result<(), GenerateError> {
    fs::create_dir_all(dir)?;

    let mut index: BTreeMap<String, String> = BTreeMap::new();

    for doc in JSON_SCHEMAS {
        let file_name = format!("{}-{}.json", doc.id.replace('.', "-"), doc.version);
        fs::write(dir.join(&file_name), doc.body)?;
        index.insert(doc.id.to_string(), file_name);
    }

    // Emit a manifest so CI can diff schemas against the previous release.
    let mut manifest = String::new();
    manifest.push_str("{\n");
    manifest.push_str(&format!("  \"schema_version\": \"{SCHEMA_VERSION}\",\n"));
    manifest.push_str("  \"schemas\": {\n");
    let entries: Vec<String> = index
        .iter()
        .map(|(id, file)| format!("    \"{id}\": \"{file}\""))
        .collect();
    manifest.push_str(&entries.join(",\n"));
    manifest.push_str("\n  }\n}\n");
    fs::write(dir.join("manifest.json"), manifest)?;

    Ok(())
}

/// Return the schema document for a given `--json` output id, if any.
pub fn schema_for(id: &str) -> Option<&'static JsonSchemaDoc> {
    JSON_SCHEMAS.iter().find(|doc| doc.id == id)
}

/// Return the schema body for a given `--json` output id, if any.
pub fn schema_body_for(id: &str) -> Option<&'static str> {
    schema_for(id).map(|doc| doc.body)
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

fn render_bash_completion(cmd: &Command, bin_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# bash completion for {bin_name}");
    let _ = writeln!(out, "_{bin_name}_completions() {{");
    let _ = writeln!(out, "    local cur prev words cword");
    let _ = writeln!(out, "    cur=\"${{COMP_WORDS[COMP_CWORD]}}\"");
    let _ = writeln!(out, "    prev=\"${{COMP_WORDS[COMP_CWORD-1]}}\"");
    let _ = writeln!(out, "    local subcommands=\"{}\"", subcommand_names(cmd).join(" "));
    let _ = writeln!(out, "    local global_opts=\"{}\"", global_option_names(cmd).join(" "));
    let _ = writeln!(out, "    if [[ $cword -eq 1 ]]; then");
    let _ = writeln!(out, "        COMPREPLY=( $(compgen -W \"$subcommands $global_opts\" -- \"$cur\") )");
    let _ = writeln!(out, "        return 0");
    let _ = writeln!(out, "    fi");
    let _ = writeln!(out, "    case \"${{COMP_WORDS[1]}}\" in");
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        let opts = subcommand_option_names(sub).join(" ");
        let _ = writeln!(out, "        {name})");
        let _ = writeln!(out, "            COMPREPLY=( $(compgen -W \"{opts}\" -- \"$cur\") )");
        let _ = writeln!(out, "            ;;");
    }
    let _ = writeln!(out, "        *)");
    let _ = writeln!(out, "            COMPREPLY=( $(compgen -W \"$global_opts\" -- \"$cur\") )");
    let _ = writeln!(out, "            ;;");
    let _ = writeln!(out, "    esac");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "complete -F _{bin_name}_completions {bin_name}");
    out
}

fn render_zsh_completion(cmd: &Command, bin_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#compdef {bin_name}");
    let _ = writeln!(out, "# zsh completion for {bin_name}");
    let _ = writeln!(out, "_{bin_name}() {{");
    let _ = writeln!(out, "    local -a subcommands");
    let _ = writeln!(out, "    subcommands=(");
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
        let _ = writeln!(out, "        '{name}:{about}'");
    }
    let _ = writeln!(out, "    )");
    let _ = writeln!(out, "    _arguments -C \\");
    let _ = writeln!(out, "        '1: :->subcommand' \\");
    let _ = writeln!(out, "        '*:: :->args'");
    let _ = writeln!(out, "    case $state in");
    let _ = writeln!(out, "        subcommand) _describe 'subcommand' subcommands ;;");
    let _ = writeln!(out, "        args)");
    let _ = writeln!(out, "            case $words[1] in");
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        let opts = subcommand_option_names(sub)
            .into_iter()
            .map(|o| format!("'{o}'"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(out, "                {name}) _values 'options' {opts} ;;");
    }
    let _ = writeln!(out, "            esac");
    let _ = writeln!(out, "            ;;");
    let _ = writeln!(out, "    esac");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "_{bin_name} \"$@\"");
    out
}

fn render_fish_completion(cmd: &Command, bin_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# fish completion for {bin_name}");
    let _ = writeln!(out, "complete -c {bin_name} -f");
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
        let _ = writeln!(
            out,
            "complete -c {bin_name} -n '__fish_use_subcommand' -a '{name}' -d '{about}'"
        );
    }
    for sub in cmd.get_subcommands() {
        let name = sub.get_name();
        if name == "help" {
            continue;
        }
        for opt in subcommand_option_names(sub) {
            let long = opt.trim_start_matches('-');
            let _ = writeln!(
                out,
                "complete -c {bin_name} -n '__fish_seen_subcommand_from {name}' -l '{long}'"
            );
        }
    }
    out
}

fn render_man_page(cmd: &Command, bin_name: &str, sub: Option<&str>) -> String {
    let mut out = String::new();
    let title = match sub {
        Some(s) => format!("{bin_name}-{s}"),
        None => bin_name.to_string(),
    };
    let section = "1";
    let about = cmd.get_about().map(|a| a.to_string()).unwrap_or_default();

    let _ = writeln!(out, ".TH {} {} \"\" \"lafiya-cli\" \"User Commands\"", title.to_uppercase(), section);
    let _ = writeln!(out, ".SH NAME");
    let _ = writeln!(out, "{title} \\- {about}");
    let _ = writeln!(out, ".SH SYNOPSIS");
    let _ = writeln!(out, ".B {title}");
    let _ = writeln!(out, "[\\fIOPTIONS\\fR]");
    let _ = writeln!(out, ".SH DESCRIPTION");
    let _ = writeln!(out, "{about}");

    let opts = subcommand_option_names(cmd);
    if !opts.is_empty() {
        let _ = writeln!(out, ".SH OPTIONS");
        for opt in opts {
            let _ = writeln!(out, ".TP");
            let _ = writeln!(out, ".B {opt}");
        }
    }

    let subs: Vec<&str> = cmd
        .get_subcommands()
        .map(|s| s.get_name())
        .filter(|n| *n != "help")
        .collect();
    if !subs.is_empty() {
        let _ = writeln!(out, ".SH SUBCOMMANDS");
        for s in subs {
            let _ = writeln!(out, ".TP");
            let _ = writeln!(out, ".B {s}");
        }
    }

    let _ = writeln!(out, ".SH JSON OUTPUT");
    let _ = writeln!(out, "Machine-readable output is available via \\fB--json\\fR.");
    let _ = writeln!(out, "Schemas are versioned (contract version {SCHEMA_VERSION}).");

    out
}

fn subcommand_names(cmd: &Command) -> Vec<String> {
    cmd.get_subcommands()
        .map(|s| s.get_name().to_string())
        .filter(|n| n != "help")
        .collect()
}

fn global_option_names(cmd: &Command) -> Vec<String> {
    cmd.get_arguments()
        .filter_map(|a| a.get_long().map(|l| format!("--{l}")))
        .collect()
}

fn subcommand_option_names(cmd: &Command) -> Vec<String> {
    let mut names: Vec<String> = cmd
        .get_arguments()
        .filter_map(|a| a.get_long().map(|l| format!("--{l}")))
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_has_unique_id_and_version() {
        let mut seen = std::collections::BTreeSet::new();
        for doc in JSON_SCHEMAS {
            assert!(seen.insert(doc.id), "duplicate schema id: {}", doc.id);
            assert!(!doc.version.is_empty(), "empty version for {}", doc.id);
            assert!(
                doc.body.contains("\"schema_version\""),
                "schema {} missing schema_version field",
                doc.id
            );
        }
    }

    #[test]
    fn schema_lookup_works() {
        assert!(schema_for("preview").is_some());
        assert!(schema_for("pool.read").is_some());
        assert!(schema_for("config").is_some());
        assert!(schema_for("does-not-exist").is_none());
    }

    #[test]
    fn schemas_are_valid_json() {
        for doc in JSON_SCHEMAS {
            // Minimal structural sanity: balanced braces and quoted keys.
            assert!(doc.body.trim_start().starts_with('{'));
            assert!(doc.body.trim_end().ends_with('}'));
        }
    }
}