//! A pack's history as checkpoints someone declared (#122).
//!
//! Between two builds the mirror kept exactly one thing: the config currently
//! on disk. A curator could work for days -- add mods, rewrite the card, repin
//! a version -- and nothing recorded that a change happened, who made it, or
//! why. The only checkpoints were published builds, which is a release
//! granularity: reverting to one throws away every edit since.
//!
//! A commit is a snapshot, an author, a message, and a parent. Deliberately not
//! a version control system: history is linear, one line per pack, with no
//! branches, no merge of divergent lines, no rebase and no amend. That is scope
//! and not prophecy -- concurrent edits already merge live (#115), and a pack
//! that genuinely diverges is a fork, which the model carries as `fork_of`.
//! Reverting writes the old state forward as a new commit rather than rewriting
//! what came before, so nothing that was ever declared stops being true.
//!
//! **A commit stores the whole config, not a delta.** A config is tens of
//! kilobytes and the structural diff between two of them is already
//! implemented, so a delta format would be a thing to maintain for no gain at
//! this size. Hundreds of commits on one pack is tens of megabytes -- worth
//! compressing if it ever bites, not worth pre-empting.
//!
//! **The id is content-addressed**, as git's is: it covers the parent, the
//! author, the message, the timestamp and the snapshot. Two commits with the
//! same id are the same commit, and a stored commit cannot be edited into a
//! different one while keeping its name. That is what lets a build name the
//! commit it came from and have the name mean something later.
//!
//! **Metadata and snapshot are separate files.** Reading the log walks the
//! parent chain, and a log view that had to read every snapshot to show a list
//! of messages would read megabytes to render kilobytes.

use crate::domain::PackConfig;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One declared checkpoint. The snapshot it refers to lives beside it; this is
/// what the log shows and what a build records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct Commit {
    /// Content address over everything below plus the snapshot.
    pub id: String,
    /// The commit this one follows. Absent only on a pack's first commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent: Option<String>,
    /// Who pressed it. One name, because one person declared it.
    pub author: String,
    pub message: String,
    /// RFC 3339, UTC.
    pub at: String,
    /// `edit_rev` of the snapshot, so "has anything changed since?" is a string
    /// comparison against the live config rather than a structural walk.
    pub config_rev: String,
    /// Everyone whose saved work this commit takes in, the author included.
    ///
    /// The state is shared and merges live, so there are no separate change
    /// sets to divide between people: whoever presses it signs it, and everyone
    /// who saved since the last commit is named in it. Naming them here rather
    /// than deriving it later is what makes attribution a record of what
    /// happened instead of a guess made afterwards.
    #[serde(default)]
    pub contributors: Vec<String>,
}

/// A commit's snapshot, stored beside its metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitSnapshot {
    pub config: PackConfig,
}

/// What the panel needs to decide whether a commit is worth offering: where the
/// history is now, and whether the live config has moved off it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct CommitStatus {
    /// The newest commit, absent on a pack that has never committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<Commit>,
    /// Fields that differ between the live config and `head`'s snapshot. Zero
    /// means the working state is exactly the last checkpoint, which is the
    /// only state in which a build needs no new commit.
    #[ts(type = "number")]
    pub uncommitted: usize,
    /// Who has saved since `head`, so the commit dialog can name them before it
    /// is pressed rather than after.
    #[serde(default)]
    pub pending_authors: Vec<String>,
}

/// Build a commit for `config`, following `parent`.
///
/// The id is computed here rather than assigned by the store, so a commit is
/// the same commit whoever writes it, and a snapshot that does not hash to its
/// own name is detectably wrong.
pub fn make_commit(
    config: &PackConfig,
    parent: Option<String>,
    author: &str,
    message: &str,
    contributors: Vec<String>,
    at: String,
) -> Result<(Commit, CommitSnapshot), serde_json::Error> {
    let config_rev = config.edit_rev()?;
    let snapshot_bytes = serde_json::to_vec(config)?;

    // A newline-joined preimage rather than a serialized struct: the id must not
    // change because a field was added to `Commit` or because serde reordered
    // one. What it covers is stated here, in one place, and nowhere else.
    let mut preimage = Vec::new();
    preimage.extend_from_slice(parent.as_deref().unwrap_or("").as_bytes());
    preimage.push(b'\n');
    preimage.extend_from_slice(author.as_bytes());
    preimage.push(b'\n');
    preimage.extend_from_slice(at.as_bytes());
    preimage.push(b'\n');
    preimage.extend_from_slice(message.as_bytes());
    preimage.push(b'\n');
    preimage.extend_from_slice(&snapshot_bytes);

    Ok((
        Commit {
            id: crate::storage::sha1_hex(&preimage),
            parent,
            author: author.to_string(),
            message: message.to_string(),
            at,
            config_rev,
            contributors,
        },
        CommitSnapshot {
            config: config.clone(),
        },
    ))
}

/// Every field whose value differs between two configs, addressed by path.
///
/// Counted rather than rendered by the caller today, but a count is the thing
/// that must not lie: "47 changes since the last commit" is what tells someone
/// a checkpoint is worth making. Arrays are compared by position and reported
/// at the row, because a mod row is the unit a person edits -- `mods.3` is an
/// address someone can act on where `mods` is not.
pub fn changed_paths(before: &serde_json::Value, after: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    walk(before, after, String::new(), &mut out);
    out
}

fn walk(before: &serde_json::Value, after: &serde_json::Value, at: String, out: &mut Vec<String>) {
    use serde_json::Value;
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let null = Value::Null;
                let path = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                walk(
                    a.get(key).unwrap_or(&null),
                    b.get(key).unwrap_or(&null),
                    path,
                    out,
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) if x == y => {}
                    _ => out.push(format!("{at}.{i}")),
                }
            }
        }
        _ => {
            if at.is_empty() {
                // two configs that share no shape at all; report the whole thing
                // rather than nothing, so a count of zero always means "equal"
                out.push("config".to_string());
            } else {
                out.push(at);
            }
        }
    }
}

/// How far the live config has moved off a commit's snapshot.
pub fn uncommitted(head: Option<&PackConfig>, live: &PackConfig) -> usize {
    let Some(head) = head else {
        // Nothing committed yet: everything there is is uncommitted, but a
        // field count would read as noise on a pack that has simply never used
        // history. One outstanding change -- the pack itself.
        return 1;
    };
    let (Ok(a), Ok(b)) = (serde_json::to_value(head), serde_json::to_value(live)) else {
        return 0;
    };
    changed_paths(&a, &b).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn equal_configs_have_nothing_uncommitted() {
        assert!(changed_paths(&v(r#"{"a":1,"b":[1,2]}"#), &v(r#"{"a":1,"b":[1,2]}"#)).is_empty());
    }

    #[test]
    fn a_scalar_is_reported_at_its_own_path() {
        assert_eq!(
            changed_paths(&v(r#"{"a":1,"b":2}"#), &v(r#"{"a":9,"b":2}"#)),
            vec!["a"]
        );
    }

    #[test]
    fn a_row_is_reported_as_the_row_not_its_fields() {
        // what a person edits is the mod row; "mods.1" is an address they can act
        // on where "mods.1.filename" buries the row in its own detail
        assert_eq!(
            changed_paths(
                &v(r#"{"mods":[{"f":"a"},{"f":"b"}]}"#),
                &v(r#"{"mods":[{"f":"a"},{"f":"c"}]}"#)
            ),
            vec!["mods.1"]
        );
    }

    #[test]
    fn an_added_or_removed_row_counts_once() {
        assert_eq!(
            changed_paths(&v(r#"{"m":[1,2]}"#), &v(r#"{"m":[1,2,3]}"#)),
            vec!["m.2"]
        );
        assert_eq!(
            changed_paths(&v(r#"{"m":[1,2,3]}"#), &v(r#"{"m":[1,2]}"#)),
            vec!["m.2"]
        );
    }

    #[test]
    fn a_missing_key_differs_from_a_present_one() {
        assert_eq!(changed_paths(&v(r#"{"a":1}"#), &v(r#"{}"#)), vec!["a"]);
    }

    #[test]
    fn nested_paths_are_addresses() {
        assert_eq!(
            changed_paths(&v(r#"{"meta":{"d":"x"}}"#), &v(r#"{"meta":{"d":"y"}}"#)),
            vec!["meta.d"]
        );
    }
}
