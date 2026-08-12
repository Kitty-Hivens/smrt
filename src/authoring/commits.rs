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

use super::configdiff::ConfigChange;
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
    /// What a commit would record: every difference between `head`'s snapshot
    /// and the live config, as rows. Empty on a pack with no history -- there is
    /// nothing to compare against, and `uncommitted` says so on its own.
    #[serde(default)]
    pub changes: Vec<ConfigChange>,
    /// How many of them there are. Zero means the working state is exactly the
    /// last checkpoint, which is the only state in which a build needs no new
    /// commit. It is the length of `changes` wherever there is a `head` to
    /// compare against, so a number can never disagree with the list beside it;
    /// on a pack that has never committed it is 1 with nothing listed, because
    /// the outstanding change is the pack itself and there is nothing to
    /// compare it against.
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
