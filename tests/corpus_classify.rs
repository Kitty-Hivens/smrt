//! Golden-corpus classification runner (stage D acceptance). Reads the labels
//! in `testdata/side-labels.toml` and grades the deployed classification
//! cascade on the pinned jars: Modrinth environment flags for a
//! Modrinth-identified jar, the bytecode classifier for the rest -- exactly
//! the priority the decision layer applies.
//!
//! Bars held (the run prints every miss with its evidence):
//!   - invariant: no client-labeled jar classifies must_match, in either branch;
//!   - cascade category agreement >= 90% over the whole corpus;
//!   - bytecode-only agreement >= 90% over the jars that have no Modrinth
//!     identity (the population the bytecode branch actually decides in prod).
//!
//! An `unclassified` verdict against an `optional_both` label counts as
//! agreement: refusing to guess degrades to a toggleable optional by design
//! (owner decision, stage D.2), which is the same presence the label expects.
//!
//! Ignored by default: the jars live outside the repo (~370 MB). Fetch them
//! with `testdata/corpus/fetch.py`, then run
//!
//!   SMRT_CORPUS_DIR=<dir with jars/ + the modrinth json snapshots> \
//!     cargo test --test corpus_classify -- --ignored --nocapture

use smrt::authoring::bytecode::JarKind;
use smrt::authoring::harvest::read_jar;
use smrt::domain::{MatchPolicy, SideClass, side_from_modrinth_env};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(serde::Deserialize)]
struct Labels {
    mods: Vec<Label>,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct Label {
    filename: String,
    pack: String,
    sha1: String,
    side: String,
    policy: String,
    kind: String,
    category: String,
    note: String,
}

/// The presence category a graph-free classification produces.
fn category(side: Option<SideClass>, policy: Option<MatchPolicy>) -> &'static str {
    match (side, policy) {
        (Some(SideClass::Client), _) => "optional_client",
        (Some(SideClass::Server), _) => "optional_server",
        (_, Some(MatchPolicy::MustMatch)) => "required",
        (Some(SideClass::Both), Some(MatchPolicy::Tolerant)) => "optional_both",
        _ => "unclassified",
    }
}

/// Agreement rule: exact category, or the designed unclassified->optional
/// degradation against an optional_both label.
fn agrees(got: &str, label: &str) -> bool {
    got == label || (got == "unclassified" && label == "optional_both")
}

#[test]
#[ignore = "needs the fetched corpus (SMRT_CORPUS_DIR)"]
fn corpus_classification_meets_the_acceptance_bar() {
    let corpus = PathBuf::from(
        std::env::var("SMRT_CORPUS_DIR").expect("SMRT_CORPUS_DIR must point at the corpus dir"),
    );
    let labels: Labels = toml::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/side-labels.toml"
        ))
        .expect("labels file"),
    )
    .expect("labels parse");
    let json = |name: &str| -> serde_json::Value {
        serde_json::from_slice(&fs::read(corpus.join(name)).expect(name)).expect(name)
    };
    let versions = json("modrinth_versions.json");
    let projects = json("modrinth_projects.json");
    let mods_meta = json("mods_meta.json");

    // sha1 -> Modrinth project id, the way prod identifies: the sha1 lookup,
    // else the manifest-declared source.
    let project_for = |sha: &str| -> Option<String> {
        if let Some(v) = versions.get(sha) {
            return v["project_id"].as_str().map(str::to_string);
        }
        let src = &mods_meta.get(sha)?["source"];
        (src["type"].as_str() == Some("modrinth"))
            .then(|| src["project_id"].as_str().map(str::to_string))
            .flatten()
    };
    let env_category = |pid: &str| -> Option<&'static str> {
        let p = projects.get(pid)?;
        let (side, policy) =
            side_from_modrinth_env(p["client_side"].as_str()?, p["server_side"].as_str()?)?;
        Some(category(Some(side), Some(policy)))
    };

    let mut cascade: HashMap<&str, (usize, usize)> = HashMap::new(); // bucket -> (agree, total)
    let mut misses: Vec<String> = Vec::new();
    let mut client_violations: Vec<String> = Vec::new();
    for label in &labels.mods {
        let path = corpus.join("jars").join(format!("{}.jar", label.sha1));
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("corpus jar missing for {}: {e}", label.filename));
        let r = read_jar(&bytes);
        let bytecode_cat = match r.bytecode.kind {
            Some(JarKind::Coremod) | Some(JarKind::Library) => "coremod",
            _ => category(r.bytecode.side, r.bytecode.match_policy),
        };
        let project = project_for(&label.sha1);
        let got = project
            .as_deref()
            .and_then(env_category)
            .unwrap_or(bytecode_cat);
        let bucket = if project.is_some() {
            "modrinth-identified"
        } else {
            "bytecode-only"
        };

        if label.category == "optional_client" && (got == "required" || bytecode_cat == "required")
        {
            client_violations.push(format!(
                "{}: must_match verdict on a client-labeled jar (cascade {got}, bytecode {bytecode_cat})",
                label.filename
            ));
        }
        let e = cascade.entry(bucket).or_default();
        e.1 += 1;
        if agrees(got, &label.category) {
            e.0 += 1;
        } else {
            misses.push(format!(
                "[{bucket}] {}: labeled {}, cascade {} (bytecode {}; side {:?} policy {:?} kind {:?}) evidence {:?}",
                label.filename,
                label.category,
                got,
                bytecode_cat,
                r.bytecode.side,
                r.bytecode.match_policy,
                r.bytecode.kind,
                r.bytecode.evidence,
            ));
        }
    }

    let (mut agree_all, mut total_all) = (0, 0);
    for (bucket, (agree, total)) in &cascade {
        println!("{bucket}: {agree}/{total}");
        agree_all += agree;
        total_all += total;
    }
    for m in &misses {
        println!("MISS {m}");
    }
    assert!(
        client_violations.is_empty(),
        "client-labeled jars classified must_match (the invariant):\n{}",
        client_violations.join("\n")
    );
    assert!(
        agree_all * 10 >= total_all * 9,
        "cascade agreement {agree_all}/{total_all} is below the 90% bar; misses above"
    );
    let (bc_agree, bc_total) = cascade.get("bytecode-only").copied().unwrap_or((0, 0));
    assert!(
        bc_agree * 10 >= bc_total * 9,
        "bytecode-only agreement {bc_agree}/{bc_total} is below the 90% bar; misses above"
    );
}
