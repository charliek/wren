//! The state-file validator of released Wren, copied verbatim from v0.1.1
//! (`git show v0.1.1:crates/wren-core/src/workspace_state.rs`, lines 12-16,
//! 459-464 and 511-656). A build that rejects a file disables the whole file,
//! so every state file this version writes must pass it. Never edit the
//! copied lines; only `accepts` below is new.

use std::collections::BTreeSet;
use std::path::Path;

use toml::{Table, Value};
use wren_core::fs_store::RelPath;

/// Whether released Wren 0.1.1 would load these bytes as writable metadata.
pub fn accepts(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{STATE_PATH} is not UTF-8"))?;
    let table = text
        .parse::<Table>()
        .map_err(|error| format!("invalid TOML in {STATE_PATH}: {}", error.message()))?;
    validate(&table)
}

const STATE_PATH: &str = ".wren/workspace.toml";
const SCHEMA_VERSION: i64 = 1;
/// Pane widths live under `[widths.columns]`, the name from when Wren had a
/// second layout, so widths saved by earlier versions still apply.
const WIDTHS_TABLE: &str = "columns";

#[derive(Clone, Copy)]
enum Reference {
    Any,
    Note,
    Container,
}

fn parse_path(path: &str, kind: Reference) -> Result<RelPath, String> {
    // Validate the portable spelling before Path normalizes away components or
    // treats a Windows absolute path as an ordinary Unix filename.
    if path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':')
        || path
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return Err("Metadata contains an invalid workspace-relative reference".into());
    }
    let depth = path.split('/').count();
    let markdown = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    match kind {
        Reference::Note if depth < 2 || !markdown => {
            return Err("History and last-open-note references must name Markdown notes".into());
        }
        // Directories may themselves end in .md. Session validates item kinds
        // against the scanned model; a filename extension is not a type check.
        Reference::Container if depth > 2 => {
            return Err(
                "Ordering and sort references must name sections or groups, not notes".into(),
            );
        }
        _ => {}
    }
    RelPath::new(path)
        .map_err(|_| "Metadata contains an invalid workspace-relative reference".into())
}

fn check_sort(mode: &str) -> Result<(), String> {
    match mode {
        "modified" | "title" => Ok(()),
        _ => Err("The note sort mode must be modified or title".into()),
    }
}

/// Each browser pane's key with its default, minimum, and maximum width.
const PANE_WIDTHS: [(&str, u16, u16, u16); 3] = [
    ("sections", 176, 140, 360),
    ("groups", 192, 150, 400),
    ("notes", 256, 200, 480),
];

fn width_limits(pane: &str) -> Option<(u16, u16, u16)> {
    PANE_WIDTHS
        .iter()
        .find(|(key, ..)| *key == pane)
        .map(|&(_, default, min, max)| (default, min, max))
}

fn validate(table: &Table) -> Result<(), String> {
    if let Some(version) = table.get("schema_version") {
        if version.as_integer() != Some(SCHEMA_VERSION) {
            return Err("Unsupported workspace metadata schema_version (expected 1)".into());
        }
    }
    for (field, kind) in [
        ("selection", Reference::Any),
        ("last_open_note", Reference::Note),
    ] {
        if let Some(value) = table.get(field) {
            parse_path(
                value
                    .as_str()
                    .ok_or_else(|| format!("{field} must be a workspace-relative path string"))?,
                kind,
            )?;
        }
    }
    for (field, kind) in [
        ("history", Reference::Note),
        ("recent", Reference::Note),
        ("order", Reference::Container),
    ] {
        if let Some(value) = table.get(field) {
            let entries = value
                .as_array()
                .ok_or_else(|| format!("{field} must be an array of path strings"))?;
            let mut seen = BTreeSet::new();
            for entry in entries {
                let path = parse_path(
                    entry
                        .as_str()
                        .ok_or_else(|| format!("{field} must contain only path strings"))?,
                    kind,
                )?;
                if field != "history" && !seen.insert(path) {
                    return Err(format!("{field} must not contain duplicate paths"));
                }
            }
        }
    }
    if let Some(value) = table.get("history_position") {
        let position = value
            .as_integer()
            .and_then(|position| usize::try_from(position).ok())
            .ok_or("history_position must be a nonnegative integer")?;
        let length = table
            .get("history")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if position >= length {
            return Err("history_position must name an occurrence in history".into());
        }
    }
    if let Some(value) = table.get("widths") {
        let widths = value
            .as_table()
            .ok_or_else(|| "widths must be a table".to_owned())?;
        if let Some(value) = widths.get(WIDTHS_TABLE) {
            let panes = value
                .as_table()
                .ok_or_else(|| "The column widths must be a table".to_owned())?;
            for (pane, value) in panes {
                if let Some((_, min, max)) = width_limits(pane) {
                    let width = value
                        .as_integer()
                        .ok_or_else(|| "Pane widths must be integers".to_owned())?;
                    if !(i64::from(min)..=i64::from(max)).contains(&width) {
                        return Err("A persisted pane width is outside its supported range".into());
                    }
                }
            }
        }
    }
    if let Some(value) = table.get("sort") {
        let sort = value
            .as_table()
            .ok_or_else(|| "sort must be a table keyed by section/group paths".to_owned())?;
        for (path, mode) in sort {
            parse_path(path, Reference::Container)?;
            check_sort(
                mode.as_str()
                    .ok_or_else(|| "Sort modes must be strings".to_owned())?,
            )?;
        }
    }
    Ok(())
}
