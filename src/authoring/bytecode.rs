//! Jar-level derivation: reduce a jar's `.class` entries (via [`classfile`]) to
//! the facts the registry stores -- which package prefixes the jar OWNS, which
//! other mods' prefixes it references (hard vs soft), and its client/server side.
//!
//! Hard vs soft is decided at class granularity: a referenced prefix is a *soft*
//! (optional) dependency only when every class that references it is conditional
//! integration code (an `isModLoaded` guard / `@Optional` / plugin marker). One
//! unconditional reference makes it a *hard* dependency. The package->owner join
//! that turns these prefixes into edges lives in the registry (`harvest`), since
//! it needs the index built from every jar.

use super::archive::read_zip_entry;
use super::classfile::{ClassInfo, parse_class};
use std::collections::BTreeSet;
use std::io::Cursor;

/// A mod's runtime side, derived from `@Mod(clientSideOnly/serverSideOnly)`
/// (Forge 1.7-1.12) or `fabric.mod.json` `environment` (Fabric/Quilt, any
/// version). Modern Forge/NeoForge declare no standard mod-level side, so those
/// jars stay `None` -- undecided is treated as both (a content mod must match the
/// server), which is why we do not guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Both,
    Client,
    Server,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Both => "both",
            Side::Client => "client",
            Side::Server => "server",
        }
    }
}

/// The derivation facts one jar yields.
#[derive(Debug, Clone, Default)]
pub struct JarBytecode {
    /// Package prefixes this jar defines (its identity in the package index).
    pub owned: BTreeSet<String>,
    /// Referenced prefixes with at least one unconditional referencing class.
    pub hard_refs: BTreeSet<String>,
    /// Referenced prefixes referenced only from conditional (integration) classes.
    pub optional_refs: BTreeSet<String>,
    /// Derived side, or `None` when nothing in the jar decides it.
    pub side: Option<Side>,
}

/// First-segment roots too broad for a 2-segment prefix to be distinctive
/// (`com/author/mod`, not just `com/author`). Package-owning identity for these
/// needs the third segment.
const COMMON_ROOTS: &[&str] = &[
    "com", "net", "org", "io", "me", "dev", "gnu", "cpw", "cn", "ru", "pl", "fr", "de", "eu", "co",
    "uk", "tv", "xyz", "info", "mod", "mods", "cc", "gg", "app", "team", "site", "moe", "su",
];

/// Platform + common-library namespaces that are never a mod's identity. A class
/// under any of these is neither owned nor a dependency edge. Kept specific (e.g.
/// `com/google`, not all of `com`) so real mods under those roots still index.
const STOP_PREFIXES: &[&str] = &[
    "java",
    "javax",
    "jdk",
    "sun",
    "com/sun",
    "kotlin",
    "scala",
    "groovy",
    "net/minecraft",
    "net/minecraftforge",
    "cpw/mods/fml",
    "cpw/mods/modlauncher",
    "net/neoforged",
    "net/fabricmc",
    "org/quiltmc",
    "com/mojang",
    "org/lwjgl",
    "org/lwjglx",
    "com/ibm/icu",
    "org/apache",
    "org/slf4j",
    "org/objectweb/asm",
    "org/ow2/asm",
    "org/spongepowered",
    "com/google",
    "io/netty",
    "it/unimi/dsi",
    "gnu/trove",
    "org/joml",
    "com/typesafe",
    "oshi",
    "org/jline",
    "joptsimple",
    "org/checkerframework",
    "org/intellij",
    "org/jetbrains",
    "javassist",
    "paulscode",
    "com/jcraft",
];

/// Reduce a jar to its derivation facts. Best-effort: an unreadable jar or
/// unparseable class contributes nothing rather than failing.
pub fn scan_jar(jar_bytes: &[u8]) -> JarBytecode {
    let classes = read_classes(jar_bytes);
    let fabric = fabric_side(jar_bytes);
    aggregate(&classes, fabric)
}

/// Parse every `.class` entry in the jar. Non-zip / non-class / malformed entries
/// are skipped.
fn read_classes(jar_bytes: &[u8]) -> Vec<ClassInfo> {
    let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(jar_bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        if !name.ends_with(".class") {
            continue;
        }
        let size = entry.size();
        let Ok(bytes) = read_zip_entry(&mut entry, size, &name) else {
            continue;
        };
        if let Some(info) = parse_class(&bytes) {
            out.push(info);
        }
    }
    out
}

/// Fold parsed classes + a fabric-side hint into the jar's facts. Pure -- the
/// unit-tested core; `scan_jar` is only the zip-reading shell around it.
fn aggregate(classes: &[ClassInfo], fabric: Option<Side>) -> JarBytecode {
    let mut owned: BTreeSet<String> = BTreeSet::new();
    for c in classes {
        if !is_platform(&c.this_class)
            && let Some(p) = package_prefix(&c.this_class)
        {
            owned.insert(p);
        }
    }

    let mut ref_all: BTreeSet<String> = BTreeSet::new();
    let mut ref_hard: BTreeSet<String> = BTreeSet::new();
    for c in classes {
        // dedup this class's referenced prefixes before grading
        let mut prefixes: BTreeSet<String> = BTreeSet::new();
        for r in &c.referenced {
            if is_platform(r) {
                continue;
            }
            if let Some(p) = package_prefix(r) {
                prefixes.insert(p);
            }
        }
        for p in prefixes {
            if !c.conditional {
                ref_hard.insert(p.clone());
            }
            ref_all.insert(p);
        }
    }

    // subtract owned so a jar never depends on itself
    let hard_refs: BTreeSet<String> = ref_hard.difference(&owned).cloned().collect();
    let optional_refs: BTreeSet<String> = ref_all
        .difference(&ref_hard)
        .filter(|p| !owned.contains(*p))
        .cloned()
        .collect();

    JarBytecode {
        owned,
        hard_refs,
        optional_refs,
        side: derive_side(classes).or(fabric),
    }
}

/// The jar's side from any `@Mod` annotations: client-only wins if some `@Mod`
/// sets `clientSideOnly` and none sets `serverSideOnly` (and vice versa);
/// otherwise `Both`. `None` when no class carried an `@Mod`.
fn derive_side(classes: &[ClassInfo]) -> Option<Side> {
    let mut saw = false;
    let (mut client, mut server) = (false, false);
    for c in classes {
        if let Some((cl, sv)) = c.mod_sides {
            saw = true;
            client |= cl;
            server |= sv;
        }
    }
    if !saw {
        return None;
    }
    Some(match (client, server) {
        (true, false) => Side::Client,
        (false, true) => Side::Server,
        _ => Side::Both,
    })
}

/// The owning prefix for a binary class name: its package, trimmed to the first
/// two segments (three under a broad root). `None` for a default-package class.
fn package_prefix(binary: &str) -> Option<String> {
    let slash = binary.rfind('/')?;
    let pkg = &binary[..slash];
    if pkg.is_empty() {
        return None;
    }
    let segs: Vec<&str> = pkg.split('/').collect();
    let depth = if COMMON_ROOTS.contains(&segs[0]) {
        3
    } else {
        2
    };
    Some(segs[..depth.min(segs.len())].join("/"))
}

/// True when a binary name sits under a platform/library namespace (matched at a
/// `/` boundary so `net/minecraftforge` matches but `net/minecraftforgeX` does not).
fn is_platform(binary: &str) -> bool {
    STOP_PREFIXES.iter().any(|p| starts_with_segment(binary, p))
}

fn starts_with_segment(name: &str, prefix: &str) -> bool {
    name == prefix || (name.starts_with(prefix) && name.as_bytes().get(prefix.len()) == Some(&b'/'))
}

/// A `fabric.mod.json` `environment` string, if the jar carries one.
fn fabric_side(jar_bytes: &[u8]) -> Option<Side> {
    let mut zip = zip::ZipArchive::new(Cursor::new(jar_bytes)).ok()?;
    let mut entry = zip.by_name("fabric.mod.json").ok()?;
    let size = entry.size();
    let raw = read_zip_entry(&mut entry, size, "fabric.mod.json").ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    match v.get("environment")?.as_str()? {
        "*" => Some(Side::Both),
        "client" => Some(Side::Client),
        "server" => Some(Side::Server),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::classfile::fixtures::{build_class, jar};
    use super::*;

    fn ci(this: &str, refs: &[&str], conditional: bool) -> ClassInfo {
        ClassInfo {
            this_class: this.into(),
            referenced: refs.iter().map(|s| s.to_string()).collect(),
            conditional,
            mod_sides: None,
        }
    }

    #[test]
    fn package_prefix_depth_by_root() {
        assert_eq!(
            package_prefix("appeng/core/AppEng").as_deref(),
            Some("appeng/core")
        );
        assert_eq!(package_prefix("appeng/Api").as_deref(), Some("appeng"));
        // broad roots take a third segment
        assert_eq!(
            package_prefix("com/author/coolmod/Main").as_deref(),
            Some("com/author/coolmod")
        );
        assert_eq!(package_prefix("Toplevel").as_deref(), None);
    }

    #[test]
    fn platform_matches_at_segment_boundary() {
        assert!(is_platform("net/minecraft/block/Block"));
        assert!(is_platform("net/minecraftforge/fml/common/Loader"));
        assert!(is_platform("org/apache/logging/log4j/Logger"));
        // a real mod under a broad root is not platform
        assert!(!is_platform("appeng/core/AppEng"));
        assert!(!is_platform("com/author/mod/Main"));
        // no false prefix match past a segment boundary
        assert!(!is_platform("javaxtra/Foo"));
    }

    #[test]
    fn hard_reference_from_unconditional_class() {
        // ae2stuff-style: a core class references AE2 with no guard -> hard dep
        let classes = vec![ci("ae2stuff/core/Main", &["appeng/api/AEApi"], false)];
        let out = aggregate(&classes, None);
        assert!(out.hard_refs.contains("appeng/api"));
        assert!(out.optional_refs.is_empty());
        assert!(out.owned.contains("ae2stuff/core"));
    }

    #[test]
    fn soft_reference_only_from_conditional_classes() {
        // every class touching JEI is integration code -> optional dep
        let classes = vec![
            ci("mymod/Main", &["mymod/Thing"], false),
            ci("mymod/compat/JeiPlugin", &["mezz/jei/api/IModPlugin"], true),
        ];
        let out = aggregate(&classes, None);
        assert!(out.optional_refs.contains("mezz/jei"));
        assert!(out.hard_refs.is_empty(), "no unconditional JEI reference");
    }

    #[test]
    fn one_hard_reference_overrides_a_soft_one() {
        // referenced both from a guarded class AND an unguarded one -> hard wins
        let classes = vec![
            ci("mymod/compat/Guarded", &["thermal/api/Energy"], true),
            ci("mymod/core/Core", &["thermal/api/Energy"], false),
        ];
        let out = aggregate(&classes, None);
        assert!(out.hard_refs.contains("thermal/api"));
        assert!(!out.optional_refs.contains("thermal/api"));
    }

    #[test]
    fn own_and_platform_references_are_not_edges() {
        let classes = vec![ci(
            "mymod/core/Core",
            &[
                "mymod/core/Helper",
                "net/minecraft/block/Block",
                "java/util/List",
            ],
            false,
        )];
        let out = aggregate(&classes, None);
        assert!(
            out.hard_refs.is_empty(),
            "self + platform refs produce no edge"
        );
        assert!(out.optional_refs.is_empty());
    }

    #[test]
    fn mod_annotation_side_wins_over_fabric() {
        let mut c = ci("mymod/ClientMod", &[], false);
        c.mod_sides = Some((true, false));
        let out = aggregate(&[c], Some(Side::Both));
        assert_eq!(out.side, Some(Side::Client));
    }

    #[test]
    fn fabric_side_used_when_no_mod_annotation() {
        let out = aggregate(&[ci("mymod/Main", &[], false)], Some(Side::Server));
        assert_eq!(out.side, Some(Side::Server));
        // and none at all stays undecided
        assert_eq!(aggregate(&[ci("mymod/Main", &[], false)], None).side, None);
    }

    #[test]
    fn scan_jar_reads_classes_and_fabric_side() {
        let addon = build_class("ae2stuff/block/Foo", &["appeng/api/AEApi"], false, None);
        let fabric = br#"{"environment":"client"}"#;
        let bytes = jar(&[
            ("ae2stuff/block/Foo.class", &addon),
            ("fabric.mod.json", fabric),
            ("pack.png", b"not a class"),
        ]);
        let out = scan_jar(&bytes);
        assert!(out.owned.contains("ae2stuff/block"));
        assert!(out.hard_refs.contains("appeng/api"));
        assert_eq!(out.side, Some(Side::Client));
    }

    #[test]
    fn scan_jar_tolerates_non_jar_bytes() {
        let out = scan_jar(b"not a zip at all");
        assert!(out.owned.is_empty() && out.hard_refs.is_empty());
        assert_eq!(out.side, None);
    }
}
