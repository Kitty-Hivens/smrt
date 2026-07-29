//! The handshake claim a pack ships, written from what the server says (#110).
//!
//! A 1.12.2 Forge server checks the client's mod list during the FML handshake
//! and refuses a client whose list is not the one it expects. A pack whose real
//! contents differ -- a modernised client, a mod swapped for a fork -- needs the
//! client to claim the server's list rather than its own, which is what
//! hidemymods does, reading `hidemymods-spoof.json` from the instance.
//!
//! That file was typed by hand and went stale in silence: the server bumped its
//! list, the file kept claiming the old one, and the failure arrived as a
//! rejected handshake explaining nothing. The server states the list itself
//! (`mcping`, #111), so the file is derived from the answer instead.
//!
//! **The claim is not checked against reality, on purpose.** A server may demand
//! a version that exists nowhere -- a pack this mirror serves faces a server
//! wanting a library version its author never published, which is why a doctored
//! jar was being shipped to satisfy it. Copying the server's answer verbatim is
//! what lets a genuine jar ship instead, and validating the claim against the
//! registry would break exactly the case the file exists for.

use super::mcping::ServerStatus;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where the mod reads it from: the instance's working directory, which is what
/// an asset destination is relative to.
pub const SPOOF_DEST: &str = "hidemymods-spoof.json";

/// One claimed mod. `id` is the Forge mod id the server named, carried through
/// untouched -- it came from the server's own handshake, so it already is what
/// the server expects, and normalising it could only break a match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct SpoofMod {
    pub id: String,
    pub version: String,
}

/// The file itself. Field order matters to nobody, but the order of `mods` is
/// preserved on the wire, so it is preserved here: the list is the server's,
/// in the server's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct Spoof {
    pub mods: Vec<SpoofMod>,
}

/// Where a claim came from, so a spoof is traceable to a server that actually
/// gave this answer rather than being an anonymous assertion. Also what makes
/// staleness answerable: ask the same server again and compare.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "bindings/")]
pub struct SpoofSource {
    pub host: String,
    #[ts(type = "number")]
    pub port: u16,
    /// The server version string it reported alongside the list.
    pub server_version: String,
    pub asked_at: String,
}

/// Build the claim from a status answer.
///
/// Refuses a server that does not advertise. "No mods" and "will not say" are
/// different answers, and a spoof built from silence would be a guess wearing
/// the shape of one -- shipped to every player, who would then be refused by the
/// handshake with no idea why.
pub fn spoof_from_status(status: &ServerStatus) -> Result<Spoof> {
    if !status.advertises_mods {
        bail!("the server does not advertise a mod list, so there is nothing to claim");
    }
    if status.mods.is_empty() {
        bail!("the server advertises an empty mod list; a spoof of nothing is not a spoof");
    }
    Ok(Spoof {
        mods: status
            .mods
            .iter()
            .map(|m| SpoofMod {
                id: m.modid.clone(),
                version: m.version.clone(),
            })
            .collect(),
    })
}

/// What changed between the claim a pack ships and what the server says now.
/// Empty when they agree, which is the only state that is not worth reporting.
pub fn drift(shipped: &Spoof, current: &Spoof) -> Vec<String> {
    let mut out = Vec::new();
    for want in &current.mods {
        match shipped.mods.iter().find(|m| m.id == want.id) {
            None => out.push(format!(
                "{} {} is expected and not claimed",
                want.id, want.version
            )),
            Some(have) if have.version != want.version => out.push(format!(
                "{} is claimed as {} and expected as {}",
                want.id, have.version, want.version
            )),
            Some(_) => {}
        }
    }
    for have in &shipped.mods {
        if !current.mods.iter().any(|m| m.id == have.id) {
            out.push(format!(
                "{} {} is claimed and no longer expected",
                have.id, have.version
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::mcping::ServerMod;

    fn status(mods: &[(&str, &str)], advertises: bool) -> ServerStatus {
        ServerStatus {
            version: "1.12.2".into(),
            mods: mods
                .iter()
                .map(|(id, v)| ServerMod {
                    modid: (*id).into(),
                    version: (*v).into(),
                })
                .collect(),
            advertises_mods: advertises,
        }
    }

    // The order is the server's: the list goes out in the order it is written,
    // and reordering it would be a different claim.
    #[test]
    fn the_claim_is_the_servers_list_in_the_servers_order() {
        let spoof = spoof_from_status(&status(
            &[
                ("appliedenergistics2", "rv6-stable-7"),
                ("jei", "4.16.1.301"),
            ],
            true,
        ))
        .unwrap();
        assert_eq!(spoof.mods[0].id, "appliedenergistics2");
        assert_eq!(spoof.mods[1].id, "jei");
        assert_eq!(spoof.mods[0].version, "rv6-stable-7");
    }

    // The case the file exists for: a server demanding a version its author
    // never published. Checking the claim against reality would break exactly
    // this, and shipping a doctored jar to satisfy it is what that used to cost.
    #[test]
    fn a_version_that_exists_nowhere_is_claimed_verbatim() {
        let spoof = spoof_from_status(&status(&[("autoreglib", "33")], true)).unwrap();
        assert_eq!(spoof.mods[0].version, "33");
    }

    // Ids are the server's own strings. Lowercasing or trimming them could only
    // turn a working match into a failing one.
    #[test]
    fn ids_are_carried_through_untouched() {
        let spoof = spoof_from_status(&status(&[("Botania", "r1.10-364")], true)).unwrap();
        assert_eq!(spoof.mods[0].id, "Botania");
    }

    // Silence is not an empty list. A spoof built from a server that will not
    // say would be shipped to every player and refused by the handshake, with
    // nothing to explain why.
    #[test]
    fn a_server_that_will_not_say_yields_no_claim() {
        let err = spoof_from_status(&status(&[], false))
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not advertise"), "got {err}");
        let empty = spoof_from_status(&status(&[], true))
            .unwrap_err()
            .to_string();
        assert!(empty.contains("empty"), "got {empty}");
    }

    // The file serializes as hidemymods reads it: an object with `mods`, each
    // entry `id` and `version`. Pinned, because a field rename here is a pack
    // that cannot join and a mod that falls back to passthrough in silence.
    #[test]
    fn the_file_has_the_shape_the_mod_reads() {
        let spoof = spoof_from_status(&status(&[("jei", "4.16.1.301")], true)).unwrap();
        let json = serde_json::to_string(&spoof).unwrap();
        assert_eq!(json, r#"{"mods":[{"id":"jei","version":"4.16.1.301"}]}"#);
    }

    #[test]
    fn drift_names_what_moved_and_says_nothing_when_it_agrees() {
        let shipped =
            spoof_from_status(&status(&[("jei", "4.15.0"), ("gone", "1.0")], true)).unwrap();
        let current =
            spoof_from_status(&status(&[("jei", "4.16.1.301"), ("added", "2.0")], true)).unwrap();

        let moved = drift(&shipped, &current);
        assert_eq!(moved.len(), 3, "{moved:?}");
        let all = moved.join("\n");
        assert!(all.contains("jei is claimed as 4.15.0 and expected as 4.16.1.301"));
        assert!(all.contains("added 2.0 is expected and not claimed"));
        assert!(all.contains("gone 1.0 is claimed and no longer expected"));

        assert!(drift(&current, &current).is_empty());
    }
}
