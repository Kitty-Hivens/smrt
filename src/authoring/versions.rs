//! The lists the editor offers instead of a text box (#126).
//!
//! A pack's Minecraft version was typed by hand, so a typo travelled into the
//! manifest and announced itself as a launcher that would not start. The list
//! behind it is real and public, and this holds a copy of it.
//!
//! Held rather than proxied, for two reasons. Opening the editor must not depend
//! on somebody else's service being up -- and when it is down the honest answer
//! is the list as it was last known, not an empty picker, which is the same
//! shape the mod search already takes on a Modrinth outage. And the list changes
//! a few times a month, so asking per editor-open would be a request per
//! keystroke's worth of value.

use super::modrinth::{GameVersion, Modrinth};
use crate::storage::Storage;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use ts_rs::TS;

/// How stale a cached list may be before the next ask refreshes it. Minecraft
/// releases a few times a year and snapshots weekly; a day late is a list that
/// is still right about everything anyone is building against.
const MAX_AGE_SECS: u64 = 6 * 60 * 60;

/// Cache file name for the Minecraft list.
const MINECRAFT_LIST: &str = "minecraft-versions";

/// A cached list, with the answer to "how old is this?" rather than only the
/// list -- a stale list is worth serving and worth admitting to.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct MinecraftVersions {
    pub versions: Vec<GameVersion>,
    /// When the mirror last heard this from upstream (RFC 3339), for whoever is
    /// reading the answer.
    pub fetched_at: String,
    /// The same instant as a number, which is what freshness is measured
    /// against. The mirror formats time and does not parse it anywhere, and a
    /// parser earned for one comparison is more surface than one integer.
    #[ts(type = "number")]
    pub fetched_unix: u64,
    /// True when upstream could not be reached and this is what was last known.
    /// The panel can say so rather than presenting old news as current.
    #[serde(default)]
    pub stale: bool,
}

pub(super) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(super) fn rfc3339(secs: u64) -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::from_unix_timestamp(secs as i64)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// Is a cached list still worth serving without asking again?
///
/// A timestamp from the future is not fresh: a clock that moved backwards would
/// otherwise pin a cache forever, and re-fetching is cheap.
pub(super) fn is_fresh(fetched_unix: u64) -> bool {
    let now = unix_now();
    fetched_unix <= now && now - fetched_unix < MAX_AGE_SECS
}

/// The Minecraft versions, from the cache when it is fresh and from upstream
/// when it is not.
///
/// An upstream failure is not an error here: the last known list is a better
/// answer than none, and the caller is told it is what it is.
pub async fn minecraft_versions(
    storage: &Arc<Storage>,
    modrinth: &Arc<Modrinth>,
) -> Result<MinecraftVersions> {
    let cached: Option<MinecraftVersions> =
        storage.load_meta_list(MINECRAFT_LIST).await.ok().flatten();
    if let Some(list) = &cached
        && is_fresh(list.fetched_unix)
    {
        return Ok(list.clone());
    }

    match modrinth.game_versions().await {
        Ok(versions) => {
            let now = unix_now();
            let fresh = MinecraftVersions {
                versions,
                fetched_at: rfc3339(now),
                fetched_unix: now,
                stale: false,
            };
            if let Err(e) = storage.save_meta_list(MINECRAFT_LIST, &fresh).await {
                tracing::warn!(error = %e, "caching the Minecraft version list failed");
            }
            Ok(fresh)
        }
        Err(e) => match cached {
            // Old news, said to be old news. An editor with a list from
            // yesterday can still pick 1.21.1; an editor with an empty picker
            // has to go back to typing, which is what this replaced.
            Some(list) => {
                tracing::warn!(error = %format!("{e:#}"), "Minecraft versions unreachable; serving the last known list");
                Ok(MinecraftVersions {
                    stale: true,
                    ..list
                })
            }
            None => Err(e).context("no cached Minecraft version list to fall back on"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_is_measured_from_when_it_was_fetched() {
        let now = unix_now();
        assert!(is_fresh(now - 60), "a minute old is fresh");
        assert!(is_fresh(now - MAX_AGE_SECS + 60), "just inside the window");
        assert!(!is_fresh(now - MAX_AGE_SECS - 60), "just outside it");
    }

    // A clock that moved backwards would otherwise pin the cache forever, and
    // asking again costs one request.
    #[test]
    fn a_timestamp_from_the_future_is_not_fresh() {
        assert!(!is_fresh(unix_now() + 3600));
    }

    // The formatted stamp is what the answer carries; it must be readable as
    // what it claims to be rather than a bare number in a string.
    #[test]
    fn the_reported_time_is_rfc3339() {
        let s = rfc3339(1_785_000_000);
        assert!(s.starts_with("2026-"), "got {s}");
        assert!(s.ends_with('Z'), "got {s}");
    }
}
