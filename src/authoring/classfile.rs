//! A minimal, dependency-free reader for JVM `.class` files -- only the facts the
//! dependency/side derivation (#40) needs, pulled straight from the bytecode so
//! the graph never relies on what a mod's author declared.
//!
//! Per class we extract:
//!   - the class's own binary name (so we learn which packages a jar OWNS), its
//!     superclass and interfaces (content-registration signals: a class
//!     extending Block/Item/Entity registers content);
//!   - every referenced type's binary name (so a reference to another mod's
//!     package becomes a dependency edge);
//!   - whether the class is *conditionally-loaded integration code* -- it guards
//!     on a `Loader.isModLoaded` check, or carries an `@Optional` / integration
//!     plugin marker. A reference made only from such classes is a soft (optional)
//!     dependency, not a hard one;
//!   - the `@Mod` element values, when the class carries a Forge/NeoForge `@Mod`
//!     annotation: `clientSideOnly` / `serverSideOnly` as a side hint, `modid`
//!     as identity, `acceptableRemoteVersions` as the 1.12-era server-match
//!     tolerance marker (`"*"` = the mod joins a server without it);
//!   - a class-level dist annotation (`@SideOnly` / `@OnlyIn` / `@Environment`),
//!     the per-class side marker across loader eras;
//!   - whether the class implements a coremod loading plugin, or carries a
//!     `@SidedProxy` field (a client/server split -- a both-sides shape).
//!
//! All of the above is answerable from the constant pool plus the class-level
//! annotations, so we never walk the `Code` attribute's instruction stream:
//! `isModLoaded` shows up as a `Methodref` in the pool, and the `@Optional` /
//! plugin markers show up as their annotation-descriptor `Utf8`. The tradeoff is
//! class granularity -- "this class guards on some mod" rather than "this exact
//! reference is guarded" -- which is exactly the signal #33 describes.
//!
//! Best-effort: a truncated or malformed class yields `None`, never an error, so
//! one odd jar entry is skipped rather than failing a whole harvest.

use std::collections::BTreeSet;

/// The Forge/NeoForge `@Mod` annotation descriptor (modern Forge kept the
/// 1.8-1.12 `net.minecraftforge` name; 1.7.10 spelled it `cpw.mods`; NeoForge
/// moved it). Carries `clientSideOnly` / `serverSideOnly` (1.7-1.12), `modid`,
/// and `acceptableRemoteVersions` (1.12).
const MOD_DESCRIPTORS: &[&str] = &[
    "Lnet/minecraftforge/fml/common/Mod;",
    "Lcpw/mods/fml/common/Mod;",
    "Lnet/neoforged/fml/common/Mod;",
];

/// Class-level dist annotations across loader eras: Forge 1.7/1.8-1.12
/// `@SideOnly(Side.X)`, modern Forge/NeoForge `@OnlyIn(Dist.X)`, Fabric
/// `@Environment(EnvType.X)`. The enum constant name tells the side.
const DIST_DESCRIPTORS: &[&str] = &[
    "Lcpw/mods/fml/relauncher/SideOnly;",
    "Lnet/minecraftforge/fml/relauncher/SideOnly;",
    "Lnet/minecraftforge/api/distmarker/OnlyIn;",
    "Lnet/fabricmc/api/Environment;",
];

/// `@SidedProxy` field-annotation descriptors (Forge 1.7-1.12). Presence in the
/// pool means the mod splits into client/server proxies -- a both-sides shape.
const SIDED_PROXY_DESCRIPTORS: &[&str] = &[
    "Lnet/minecraftforge/fml/common/SidedProxy;",
    "Lcpw/mods/fml/common/SidedProxy;",
];

/// Coremod loading-plugin interfaces: a class implementing one is launch-time
/// transformer code, not a mod.
const LOADING_PLUGIN_INTERFACES: &[&str] = &[
    "net/minecraftforge/fml/relauncher/IFMLLoadingPlugin",
    "cpw/mods/fml/relauncher/IFMLLoadingPlugin",
];

/// Annotation descriptors that mark a class/method as conditionally-loaded
/// integration code across loaders/eras: Forge 1.7-1.12 soft-dependency
/// `@Optional.*` (modern Forge dropped it for `isModLoaded` guards), plus the
/// common integration-plugin markers. Presence of any of these `Utf8`s in a
/// class's pool means its references to the named mod are soft, not hard. The
/// list is a curated allowlist -- extend it; a miss only over-grades a soft dep
/// to hard (the safe direction), and an authored edge overrides either way.
const OPTIONAL_MARKERS: &[&str] = &[
    "Lnet/minecraftforge/fml/common/Optional$Interface;",
    "Lnet/minecraftforge/fml/common/Optional$Method;",
    "Lnet/minecraftforge/fml/common/Optional$InterfaceList;",
    "Lcpw/mods/fml/common/Optional$Interface;",
    "Lcpw/mods/fml/common/Optional$Method;",
    "Lcpw/mods/fml/common/Optional$InterfaceList;",
    // integration plugins: the class references an API it does not require.
    "Lmezz/jei/api/JEIPlugin;",               // JEI 1.12
    "Lmezz/jei/api/JeiPlugin;",               // JEI 1.13+
    "Lmcp/mobius/waila/api/WailaPlugin;",     // Waila / Hwyla
    "Lsnownee/jade/api/WailaPlugin;",         // Jade
    "Lcrafttweaker/annotations/ZenRegister;", // CraftTweaker 1.13+
];

/// `(owner-class, method-name)` pairs whose invocation guards a code path on a
/// mod being present, across loaders and eras. A class that references any of
/// these treats its mod references as conditional.
const MOD_LOADED_GUARDS: &[(&str, &str)] = &[
    ("cpw/mods/fml/common/Loader", "isModLoaded"), // Forge 1.7.10
    ("net/minecraftforge/fml/common/Loader", "isModLoaded"), // Forge 1.8-1.12
    ("net/minecraftforge/fml/ModList", "isLoaded"), // Forge 1.13+
    ("net/neoforged/fml/ModList", "isLoaded"),     // NeoForge
    ("net/fabricmc/loader/api/FabricLoader", "isModLoaded"), // Fabric
    ("org/quiltmc/loader/api/QuiltLoader", "isModLoaded"), // Quilt
];

/// The side a class-level dist annotation pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dist {
    Client,
    Server,
}

/// The facts one `.class` yields for derivation.
#[derive(Debug, Clone)]
pub struct ClassInfo {
    /// Binary name of the class, `/`-separated (e.g. `appeng/core/AppEng`).
    pub this_class: String,
    /// Binary name of the direct superclass (`None` for Object itself or a
    /// malformed entry). A content signal when it is a Block/Item/Entity base.
    pub super_name: Option<String>,
    /// Binary names of the directly implemented interfaces.
    pub interfaces: Vec<String>,
    /// Binary names of every type this class references (owners, supertypes,
    /// field/method signature types, constant-pool class entries). Deduped.
    pub referenced: BTreeSet<String>,
    /// True when the class is conditionally-loaded integration code (guards on a
    /// mod-loaded check, or carries an `@Optional` / plugin marker).
    pub conditional: bool,
    /// `(client_side_only, server_side_only)` from an `@Mod` annotation on this
    /// class, when present.
    pub mod_sides: Option<(bool, bool)>,
    /// The `modid` element of an `@Mod` annotation on this class, when present and
    /// non-empty. A Forge mod that ships no `mcmod.info` / `mods.toml` carries its
    /// identity only here (the annotation value is a compile-time constant, so a
    /// `modid = MOD_ID` reference is already inlined to its literal in the bytes).
    pub mod_id: Option<String>,
    /// The `acceptableRemoteVersions` element of an `@Mod` annotation, when the
    /// class declares one. `"*"` is the 1.12-era "the server may lack this mod"
    /// marker (JEI-style), i.e. a tolerant match policy.
    pub acceptable_remote_versions: Option<String>,
    /// The class-level dist annotation (`@SideOnly` / `@OnlyIn` /
    /// `@Environment`), when present.
    pub dist: Option<Dist>,
    /// True when the pool carries a `@SidedProxy` descriptor -- the mod splits
    /// into client/server proxies, a both-sides shape.
    pub sided_proxy: bool,
    /// True when the class implements a coremod loading-plugin interface.
    pub loading_plugin: bool,
}

/// Constant-pool entries, reduced to what derivation reads. `Skip` is the dead
/// second slot a Long/Double occupies; `Other` is a well-formed entry whose
/// contents we do not need but whose width we must account for.
enum Cp {
    Utf8(String),
    Class(u16),            // name_index
    NameAndType(u16, u16), // name_index, descriptor_index
    Ref(u16, u16),         // class_index, name_and_type_index (Field/Method/Interface)
    Integer(i32),
    Other,
    Skip,
}

/// A big-endian byte cursor. Every read is bounds-checked and returns `None` past
/// the end, so a truncated class simply parses to `None`.
struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8]) -> Self {
        Cur { b, p: 0 }
    }
    fn u1(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    fn u2(&mut self) -> Option<u16> {
        let hi = self.u1()? as u16;
        let lo = self.u1()? as u16;
        Some((hi << 8) | lo)
    }
    fn u4(&mut self) -> Option<u32> {
        let hi = self.u2()? as u32;
        let lo = self.u2()? as u32;
        Some((hi << 16) | lo)
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.p.checked_add(n)?;
        let s = self.b.get(self.p..end)?;
        self.p = end;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.p.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        self.p = end;
        Some(())
    }
}

const MAGIC: u32 = 0xCAFE_BABE;

/// Parse a `.class` blob into the facts derivation needs. `None` for anything
/// that is not a well-formed class file.
pub fn parse_class(bytes: &[u8]) -> Option<ClassInfo> {
    let mut c = Cur::new(bytes);
    if c.u4()? != MAGIC {
        return None;
    }
    c.u2()?; // minor_version
    c.u2()?; // major_version

    // ── constant pool (1-indexed; Long/Double take two slots) ──────────────
    let cp_count = c.u2()?;
    let mut cp: Vec<Cp> = Vec::with_capacity(cp_count as usize);
    cp.push(Cp::Skip); // slot 0 is unused
    let mut idx = 1u16;
    while idx < cp_count {
        let tag = c.u1()?;
        match tag {
            1 => {
                let len = c.u2()? as usize;
                let raw = c.bytes(len)?;
                cp.push(Cp::Utf8(String::from_utf8_lossy(raw).into_owned()));
            }
            7 => cp.push(Cp::Class(c.u2()?)), // Class
            8 => {
                c.u2()?; // String -> string_index (unused)
                cp.push(Cp::Other);
            }
            9..=11 => {
                // Fieldref / Methodref / InterfaceMethodref
                let class_index = c.u2()?;
                let nt_index = c.u2()?;
                cp.push(Cp::Ref(class_index, nt_index));
            }
            12 => {
                let name_index = c.u2()?;
                let desc_index = c.u2()?;
                cp.push(Cp::NameAndType(name_index, desc_index));
            }
            3 => cp.push(Cp::Integer(c.u4()? as i32)), // Integer
            4 => {
                c.u4()?; // Float
                cp.push(Cp::Other);
            }
            5 | 6 => {
                // Long / Double: 8 bytes, and consumes TWO pool slots.
                c.u4()?;
                c.u4()?;
                cp.push(Cp::Other);
                cp.push(Cp::Skip);
                idx += 2;
                continue;
            }
            15 => {
                c.u1()?; // MethodHandle: reference_kind
                c.u2()?; // reference_index
                cp.push(Cp::Other);
            }
            16 => {
                c.u2()?; // MethodType: descriptor_index
                cp.push(Cp::Other);
            }
            17 | 18 => {
                // Dynamic / InvokeDynamic
                c.u2()?;
                c.u2()?;
                cp.push(Cp::Other);
            }
            19 | 20 => {
                c.u2()?; // Module / Package: name_index
                cp.push(Cp::Other);
            }
            _ => return None, // unknown tag -> not a class we can trust
        }
        idx += 1;
    }

    // ── header past the pool ───────────────────────────────────────────────
    c.u2()?; // access_flags
    let this_class_index = c.u2()?;
    let this_class = class_name(&cp, this_class_index)?.to_string();
    let super_index = c.u2()?;
    // Object's own super_class is 0; a bare Object super is uninformative.
    let super_name = (super_index != 0)
        .then(|| class_name(&cp, super_index).map(str::to_string))
        .flatten()
        .filter(|s| s != "java/lang/Object");
    let interfaces_count = c.u2()?;
    let mut interfaces = Vec::with_capacity(interfaces_count as usize);
    for _ in 0..interfaces_count {
        let idx = c.u2()?;
        if let Some(n) = class_name(&cp, idx) {
            interfaces.push(n.to_string());
        }
    }

    // Referenced types come from three sources, unioned:
    //   1. every Class entry in the pool (owners, supertypes, casts, `new`, ...),
    //   2. the descriptors on this class's own fields/methods,
    //   3. the descriptors behind every NameAndType (i.e. field/method refs).
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for e in &cp {
        match e {
            Cp::Class(name_index) => {
                if let Some(n) = utf8(&cp, *name_index) {
                    push_type(&mut referenced, n);
                }
            }
            Cp::NameAndType(_, desc_index) => {
                if let Some(d) = utf8(&cp, *desc_index) {
                    extract_object_types(d, &mut referenced);
                }
            }
            _ => {}
        }
    }

    // fields: {access u2, name u2, descriptor u2, attributes}
    let fields_count = c.u2()?;
    for _ in 0..fields_count {
        c.u2()?; // access_flags
        c.u2()?; // name_index
        let desc_index = c.u2()?;
        if let Some(d) = utf8(&cp, desc_index) {
            extract_object_types(d, &mut referenced);
        }
        skip_attributes(&mut c)?;
    }

    // methods: same header shape; attributes skipped wholesale.
    let methods_count = c.u2()?;
    for _ in 0..methods_count {
        c.u2()?; // access_flags
        c.u2()?; // name_index
        let desc_index = c.u2()?;
        if let Some(d) = utf8(&cp, desc_index) {
            extract_object_types(d, &mut referenced);
        }
        skip_attributes(&mut c)?;
    }

    // ── class attributes: only the annotation blocks matter ────────────────
    let mut mod_sides: Option<(bool, bool)> = None;
    let mut mod_id: Option<String> = None;
    let mut acceptable_remote_versions: Option<String> = None;
    let mut dist: Option<Dist> = None;
    let attr_count = c.u2()?;
    for _ in 0..attr_count {
        let name_index = c.u2()?;
        let len = c.u4()? as usize;
        let body = c.bytes(len)?;
        let name = utf8(&cp, name_index);
        if matches!(
            name,
            Some("RuntimeVisibleAnnotations") | Some("RuntimeInvisibleAnnotations")
        ) {
            let scanned = scan_annotations(body, &cp);
            if let Some(ann) = scanned.mod_annotation {
                mod_sides = Some(ann.sides);
                if mod_id.is_none() {
                    mod_id = ann.modid;
                }
                if acceptable_remote_versions.is_none() {
                    acceptable_remote_versions = ann.acceptable_remote_versions;
                }
            }
            if dist.is_none() {
                dist = scanned.dist;
            }
        }
    }

    let referenced_self = this_class.clone();
    referenced.remove(&referenced_self);

    let loading_plugin = interfaces
        .iter()
        .any(|i| LOADING_PLUGIN_INTERFACES.contains(&i.as_str()));
    Some(ClassInfo {
        conditional: is_conditional(&cp),
        sided_proxy: has_marker(&cp, SIDED_PROXY_DESCRIPTORS),
        loading_plugin,
        this_class,
        super_name,
        interfaces,
        referenced,
        mod_sides,
        mod_id,
        acceptable_remote_versions,
        dist,
    })
}

/// True when the pool carries any of the given annotation-descriptor `Utf8`s.
fn has_marker(cp: &[Cp], descriptors: &[&str]) -> bool {
    cp.iter().any(|e| match e {
        Cp::Utf8(s) => descriptors.contains(&s.as_str()),
        _ => false,
    })
}

/// A class is conditional integration code when its pool carries an `@Optional` /
/// plugin marker descriptor, or references a `isModLoaded`-style guard call.
/// Both are visible in the constant pool without walking any bytecode.
fn is_conditional(cp: &[Cp]) -> bool {
    for e in cp {
        match e {
            Cp::Utf8(s) if OPTIONAL_MARKERS.contains(&s.as_str()) => return true,
            Cp::Ref(class_index, nt_index) => {
                let owner = class_name(cp, *class_index);
                let method = nt_name(cp, *nt_index);
                if let (Some(owner), Some(method)) = (owner, method)
                    && MOD_LOADED_GUARDS
                        .iter()
                        .any(|(o, m)| *o == owner && *m == method)
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

// ── constant-pool resolvers ──────────────────────────────────────────────────

fn utf8(cp: &[Cp], i: u16) -> Option<&str> {
    match cp.get(i as usize)? {
        Cp::Utf8(s) => Some(s.as_str()),
        _ => None,
    }
}

fn class_name(cp: &[Cp], i: u16) -> Option<&str> {
    match cp.get(i as usize)? {
        Cp::Class(name_index) => utf8(cp, *name_index),
        _ => None,
    }
}

fn nt_name(cp: &[Cp], i: u16) -> Option<&str> {
    match cp.get(i as usize)? {
        Cp::NameAndType(name_index, _) => utf8(cp, *name_index),
        _ => None,
    }
}

fn integer(cp: &[Cp], i: u16) -> Option<i32> {
    match cp.get(i as usize)? {
        Cp::Integer(v) => Some(*v),
        _ => None,
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Skip a `{ u2 count; { u2 name; u4 len; u1 body[len] } attrs[count] }` block.
fn skip_attributes(c: &mut Cur) -> Option<()> {
    let count = c.u2()?;
    for _ in 0..count {
        c.u2()?; // attribute_name_index
        let len = c.u4()? as usize;
        c.skip(len)?;
    }
    Some(())
}

/// Record a referenced binary type name from a `Class` constant. A `Class` entry
/// is usually a bare binary name (`foo/Bar`) but an array class is a descriptor
/// (`[Lfoo/Bar;`, `[[I`); strip the array/`L;` wrappers and drop single-letter
/// primitives so only real object types land.
fn push_type(out: &mut BTreeSet<String>, raw: &str) {
    let name = raw.trim_start_matches('[');
    let name = name
        .strip_prefix('L')
        .and_then(|s| s.strip_suffix(';'))
        .unwrap_or(name);
    if name.len() > 1 {
        out.insert(name.to_string());
    }
}

/// Pull every object type (`Lbinary/Name;`) out of a field/method descriptor or
/// signature, e.g. `(Lappeng/api/A;I)Lappeng/api/B;` -> `appeng/api/A`,
/// `appeng/api/B`.
fn extract_object_types(desc: &str, out: &mut BTreeSet<String>) {
    let bytes = desc.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'L' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b';' {
                j += 1;
            }
            if j < bytes.len() && j > start {
                out.insert(desc[start..j].to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
}

/// The `@Mod` element values this module reads: the mod's `modid` (its identity
/// when no metadata file is present), its `(clientSideOnly, serverSideOnly)`
/// side flags (absent element -> false), and `acceptableRemoteVersions`.
#[derive(Default)]
struct ModAnnotation {
    modid: Option<String>,
    sides: (bool, bool),
    acceptable_remote_versions: Option<String>,
}

/// Everything one annotations attribute yields.
#[derive(Default)]
struct ScannedAnnotations {
    mod_annotation: Option<ModAnnotation>,
    dist: Option<Dist>,
}

/// Scan an annotations attribute body for the annotations derivation reads: a
/// Forge/NeoForge `@Mod` (element values) and a class-level dist marker
/// (`@SideOnly` / `@OnlyIn` / `@Environment`, whose enum constant names the
/// side).
fn scan_annotations(body: &[u8], cp: &[Cp]) -> ScannedAnnotations {
    let mut out = ScannedAnnotations::default();
    let mut c = Cur::new(body);
    let Some(num) = c.u2() else { return out };
    for _ in 0..num {
        let Some((type_index, pairs)) = read_annotation(&mut c) else {
            return out; // malformed tail: keep what was already scanned
        };
        let Some(type_desc) = utf8(cp, type_index) else {
            continue;
        };
        if MOD_DESCRIPTORS.contains(&type_desc) {
            let mut ann = ModAnnotation::default();
            for (name_index, value) in pairs {
                match (utf8(cp, name_index), value) {
                    // modid is a compile-time-constant String element (tag `s`)
                    (Some("modid"), Some(ElemValue::Prim(b's', const_index))) => {
                        ann.modid = utf8(cp, const_index)
                            .map(str::to_string)
                            .filter(|s| !s.is_empty());
                    }
                    (Some("clientSideOnly"), Some(ElemValue::Prim(b'Z', const_index))) => {
                        ann.sides.0 = integer(cp, const_index).unwrap_or(0) != 0;
                    }
                    (Some("serverSideOnly"), Some(ElemValue::Prim(b'Z', const_index))) => {
                        ann.sides.1 = integer(cp, const_index).unwrap_or(0) != 0;
                    }
                    (
                        Some("acceptableRemoteVersions"),
                        Some(ElemValue::Prim(b's', const_index)),
                    ) => {
                        ann.acceptable_remote_versions = utf8(cp, const_index)
                            .map(str::to_string)
                            .filter(|s| !s.is_empty());
                    }
                    _ => {}
                }
            }
            out.mod_annotation = Some(ann);
        } else if DIST_DESCRIPTORS.contains(&type_desc) && out.dist.is_none() {
            // the side rides in the `value` element's enum constant name:
            // Side.CLIENT / Dist.CLIENT / EnvType.CLIENT, and the server-side
            // spellings SERVER / DEDICATED_SERVER.
            for (_, value) in pairs {
                if let Some(ElemValue::Enum(const_index)) = value {
                    out.dist = match utf8(cp, const_index) {
                        Some("CLIENT") => Some(Dist::Client),
                        Some("SERVER") | Some("DEDICATED_SERVER") => Some(Dist::Server),
                        _ => None,
                    };
                }
            }
        }
    }
    out
}

/// A captured element value: a primitive/String constant (`tag`, const-pool
/// index), an enum constant (`type_name_index`, `const_name_index`), or
/// `None` for structured values we only skip past.
enum ElemValue {
    Prim(u8, u16),
    Enum(u16),
}

/// Read one `annotation` structure, capturing `(type_index, pairs)` where each
/// pair is `(element_name_index, captured_value)`. Advances past the whole
/// annotation regardless of value shapes, so callers stay aligned.
#[allow(clippy::type_complexity)]
fn read_annotation(c: &mut Cur) -> Option<(u16, Vec<(u16, Option<ElemValue>)>)> {
    let type_index = c.u2()?;
    let num_pairs = c.u2()?;
    let mut pairs = Vec::with_capacity(num_pairs as usize);
    for _ in 0..num_pairs {
        let name_index = c.u2()?;
        let value = read_element_value(c)?;
        pairs.push((name_index, value));
    }
    Some((type_index, pairs))
}

/// Read one `element_value`, advancing the cursor exactly one value. Primitive,
/// String and enum values yield their constant indices; everything structural
/// is skipped and yields `None`.
fn read_element_value(c: &mut Cur) -> Option<Option<ElemValue>> {
    let tag = c.u1()?;
    match tag {
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b's' => {
            let const_index = c.u2()?;
            Some(Some(ElemValue::Prim(tag, const_index)))
        }
        b'e' => {
            c.u2()?; // enum type_name_index (the const name alone decides)
            let const_name_index = c.u2()?;
            Some(Some(ElemValue::Enum(const_name_index)))
        }
        b'c' => {
            c.u2()?; // class_info_index
            Some(None)
        }
        b'@' => {
            read_annotation(c)?; // nested annotation
            Some(None)
        }
        b'[' => {
            let n = c.u2()?;
            for _ in 0..n {
                read_element_value(c)?;
            }
            Some(None)
        }
        _ => None,
    }
}

/// Hand-assembled `.class` bytes for tests across the authoring layer (this
/// module, `bytecode`, and `harvest`), so those tests can build real jars
/// instead of shipping binary fixtures.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::MAGIC;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    /// A tiny constant-pool + class-file assembler, just enough to drive the
    /// parser: it is the inverse of what `parse_class` reads.
    #[derive(Default)]
    pub(crate) struct ClassWriter {
        pool: Vec<u8>,
        count: u16, // next index == count + 1 (slot 0 reserved)
    }

    impl ClassWriter {
        fn add(&mut self, entry: &[u8]) -> u16 {
            self.pool.extend_from_slice(entry);
            self.count += 1;
            self.count
        }
        pub(crate) fn utf8(&mut self, s: &str) -> u16 {
            let mut e = vec![1u8];
            e.extend_from_slice(&(s.len() as u16).to_be_bytes());
            e.extend_from_slice(s.as_bytes());
            self.add(&e)
        }
        pub(crate) fn class(&mut self, binary: &str) -> u16 {
            let n = self.utf8(binary);
            let mut e = vec![7u8];
            e.extend_from_slice(&n.to_be_bytes());
            self.add(&e)
        }
        pub(crate) fn integer(&mut self, v: i32) -> u16 {
            let mut e = vec![3u8];
            e.extend_from_slice(&v.to_be_bytes());
            self.add(&e)
        }
        /// A Long constant, which occupies TWO consecutive pool slots (the second
        /// is unusable). Returns the index of the first slot.
        pub(crate) fn long(&mut self, v: i64) -> u16 {
            self.pool.push(5u8);
            self.pool.extend_from_slice(&v.to_be_bytes());
            self.count += 1;
            let idx = self.count;
            self.count += 1; // reserve the phantom second slot
            idx
        }
        pub(crate) fn name_and_type(&mut self, name: &str, desc: &str) -> u16 {
            let n = self.utf8(name);
            let d = self.utf8(desc);
            let mut e = vec![12u8];
            e.extend_from_slice(&n.to_be_bytes());
            e.extend_from_slice(&d.to_be_bytes());
            self.add(&e)
        }
        pub(crate) fn methodref(&mut self, owner: &str, name: &str, desc: &str) -> u16 {
            let c = self.class(owner);
            let nt = self.name_and_type(name, desc);
            let mut e = vec![10u8];
            e.extend_from_slice(&c.to_be_bytes());
            e.extend_from_slice(&nt.to_be_bytes());
            self.add(&e)
        }
        /// Assemble the full class: `this_class`, optional class-level annotation
        /// bytes, everything else empty.
        pub(crate) fn build(
            self,
            this_index: u16,
            object_index: u16,
            class_attrs: &[u8],
            attr_n: u16,
        ) -> Vec<u8> {
            self.build_with(this_index, object_index, &[], class_attrs, attr_n)
        }

        /// Like [`build`], with an explicit superclass and interface list.
        pub(crate) fn build_with(
            self,
            this_index: u16,
            super_index: u16,
            interfaces: &[u16],
            class_attrs: &[u8],
            attr_n: u16,
        ) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&MAGIC.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes()); // minor
            out.extend_from_slice(&52u16.to_be_bytes()); // major (Java 8)
            out.extend_from_slice(&(self.count + 1).to_be_bytes()); // cp_count
            out.extend_from_slice(&self.pool);
            out.extend_from_slice(&0x0021u16.to_be_bytes()); // access_flags
            out.extend_from_slice(&this_index.to_be_bytes());
            out.extend_from_slice(&super_index.to_be_bytes());
            out.extend_from_slice(&(interfaces.len() as u16).to_be_bytes());
            for i in interfaces {
                out.extend_from_slice(&i.to_be_bytes());
            }
            out.extend_from_slice(&0u16.to_be_bytes()); // fields_count
            out.extend_from_slice(&0u16.to_be_bytes()); // methods_count
            out.extend_from_slice(&attr_n.to_be_bytes()); // attributes_count
            out.extend_from_slice(class_attrs);
            out
        }
    }

    /// One annotation body: `type_index` + `(name_index, encoded value)` pairs.
    fn annotation(type_index: u16, pairs: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&type_index.to_be_bytes());
        b.extend_from_slice(&(pairs.len() as u16).to_be_bytes());
        for (name, value) in pairs {
            b.extend_from_slice(&name.to_be_bytes());
            b.extend_from_slice(value);
        }
        b
    }

    fn prim(tag: u8, const_index: u16) -> Vec<u8> {
        let mut b = vec![tag];
        b.extend_from_slice(&const_index.to_be_bytes());
        b
    }

    fn enum_val(type_name: u16, const_name: u16) -> Vec<u8> {
        let mut b = vec![b'e'];
        b.extend_from_slice(&type_name.to_be_bytes());
        b.extend_from_slice(&const_name.to_be_bytes());
        b
    }

    /// A RuntimeVisibleAnnotations attribute holding the given annotations.
    fn annotations_attr(w: &mut ClassWriter, annotations: &[Vec<u8>]) -> Vec<u8> {
        let name = w.utf8("RuntimeVisibleAnnotations");
        let mut body = Vec::new();
        body.extend_from_slice(&(annotations.len() as u16).to_be_bytes());
        for a in annotations {
            body.extend_from_slice(a);
        }
        let mut attrs = Vec::new();
        attrs.extend_from_slice(&name.to_be_bytes());
        attrs.extend_from_slice(&(body.len() as u32).to_be_bytes());
        attrs.extend_from_slice(&body);
        attrs
    }

    /// Everything a synthetic test class can carry; `Default` is a bare class.
    #[derive(Default)]
    pub(crate) struct ClassSpec<'a> {
        pub(crate) this: &'a str,
        pub(crate) super_name: Option<&'a str>,
        pub(crate) interfaces: &'a [&'a str],
        pub(crate) refs: &'a [&'a str],
        pub(crate) conditional: bool,
        pub(crate) mod_sides: Option<(bool, bool)>,
        pub(crate) modid: Option<&'a str>,
        pub(crate) arv: Option<&'a str>,
        /// `(annotation descriptor, enum constant name)`, e.g.
        /// `("Lnet/minecraftforge/api/distmarker/OnlyIn;", "CLIENT")`.
        pub(crate) dist: Option<(&'a str, &'a str)>,
        pub(crate) sided_proxy: bool,
    }

    /// Assemble a class from a [`ClassSpec`] -- the one builder every
    /// side/policy fixture goes through.
    pub(crate) fn build_class_spec(spec: &ClassSpec) -> Vec<u8> {
        let mut w = ClassWriter::default();
        let obj = w.class("java/lang/Object");
        let this_index = w.class(spec.this);
        let super_index = match spec.super_name {
            Some(s) => w.class(s),
            None => obj,
        };
        let iface_indexes: Vec<u16> = spec.interfaces.iter().map(|i| w.class(i)).collect();
        for r in spec.refs {
            w.class(r);
        }
        if spec.conditional {
            w.utf8("Lnet/minecraftforge/fml/common/Optional$Method;");
        }
        if spec.sided_proxy {
            w.utf8("Lnet/minecraftforge/fml/common/SidedProxy;");
        }
        let mut anns: Vec<Vec<u8>> = Vec::new();
        if spec.mod_sides.is_some() || spec.modid.is_some() || spec.arv.is_some() {
            let desc = w.utf8("Lnet/minecraftforge/fml/common/Mod;");
            let mut pairs: Vec<(u16, Vec<u8>)> = Vec::new();
            if let Some(id) = spec.modid {
                let n = w.utf8("modid");
                let v = w.utf8(id);
                pairs.push((n, prim(b's', v)));
            }
            if let Some((client, server)) = spec.mod_sides {
                let n1 = w.utf8("clientSideOnly");
                let v1 = w.integer(client as i32);
                pairs.push((n1, prim(b'Z', v1)));
                let n2 = w.utf8("serverSideOnly");
                let v2 = w.integer(server as i32);
                pairs.push((n2, prim(b'Z', v2)));
            }
            if let Some(arv) = spec.arv {
                let n = w.utf8("acceptableRemoteVersions");
                let v = w.utf8(arv);
                pairs.push((n, prim(b's', v)));
            }
            anns.push(annotation(desc, &pairs));
        }
        if let Some((desc, konst)) = spec.dist {
            let d = w.utf8(desc);
            let name = w.utf8("value");
            let tname = w.utf8("Lenum/TypeDesc;"); // parser reads only the const name
            let cname = w.utf8(konst);
            anns.push(annotation(d, &[(name, enum_val(tname, cname))]));
        }
        let (attrs, n) = if anns.is_empty() {
            (Vec::new(), 0u16)
        } else {
            (annotations_attr(&mut w, &anns), 1u16)
        };
        w.build_with(this_index, super_index, &iface_indexes, &attrs, n)
    }

    /// Build a class named `this`, referencing each of `refs`, optionally marked
    /// conditional (an `@Optional` marker in the pool) and/or carrying `@Mod`
    /// side flags. The convenience the aggregation + harvest tests build jars from.
    pub(crate) fn build_class(
        this: &str,
        refs: &[&str],
        conditional: bool,
        mod_sides: Option<(bool, bool)>,
    ) -> Vec<u8> {
        build_class_spec(&ClassSpec {
            this,
            refs,
            conditional,
            mod_sides,
            ..ClassSpec::default()
        })
    }

    /// Build a class carrying only an `@Mod(modid = "...")` annotation -- the
    /// identity path for a Forge mod that ships no metadata file.
    pub(crate) fn build_class_modid(this: &str, modid: &str) -> Vec<u8> {
        build_class_spec(&ClassSpec {
            this,
            modid: Some(modid),
            ..ClassSpec::default()
        })
    }

    /// Zip the given `(name, bytes)` entries into an in-memory jar.
    pub(crate) fn jar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            zw.start_file(*name, SimpleFileOptions::default()).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::ClassWriter;
    use super::*;

    #[test]
    fn parses_this_class_and_references() {
        let mut w = ClassWriter::default();
        let obj = w.class("java/lang/Object");
        let this = w.class("appeng/integration/Foo");
        // a plain referenced type
        w.class("appeng/api/AEApi");
        // a referenced type reachable only through a method descriptor
        w.name_and_type("doThing", "(Lmekanism/api/Gas;)V");
        let bytes = w.build(this, obj, &[], 0);

        let info = parse_class(&bytes).expect("parses");
        assert_eq!(info.this_class, "appeng/integration/Foo");
        assert!(info.referenced.contains("appeng/api/AEApi"));
        assert!(
            info.referenced.contains("mekanism/api/Gas"),
            "type from a method descriptor is picked up"
        );
        assert!(
            !info.referenced.contains("appeng/integration/Foo"),
            "self-reference is excluded"
        );
        assert!(!info.conditional);
        assert!(info.mod_sides.is_none());
    }

    #[test]
    fn reads_modid_from_mod_annotation() {
        // a Forge mod (Chisel, HatStand) whose only identity is @Mod(modid=...)
        let bytes = fixtures::build_class_modid("team/chisel/Chisel", "chisel");
        let info = parse_class(&bytes).expect("parses");
        assert_eq!(
            info.mod_id.as_deref(),
            Some("chisel"),
            "modid read from the class-level @Mod annotation"
        );
    }

    #[test]
    fn detects_ismodloaded_guard_as_conditional() {
        let mut w = ClassWriter::default();
        let obj = w.class("java/lang/Object");
        let this = w.class("mymod/compat/JeiCompat");
        w.methodref(
            "net/minecraftforge/fml/common/Loader",
            "isModLoaded",
            "(Ljava/lang/String;)Z",
        );
        let bytes = w.build(this, obj, &[], 0);
        let info = parse_class(&bytes).expect("parses");
        assert!(
            info.conditional,
            "isModLoaded guard marks the class conditional"
        );
    }

    #[test]
    fn detects_fabric_and_neoforge_guards_as_conditional() {
        for (owner, method) in [
            ("net/fabricmc/loader/api/FabricLoader", "isModLoaded"),
            ("org/quiltmc/loader/api/QuiltLoader", "isModLoaded"),
            ("net/neoforged/fml/ModList", "isLoaded"),
        ] {
            let mut w = ClassWriter::default();
            let obj = w.class("java/lang/Object");
            let this = w.class("mymod/compat/Integration");
            w.methodref(owner, method, "(Ljava/lang/String;)Z");
            let bytes = w.build(this, obj, &[], 0);
            let info = parse_class(&bytes).expect("parses");
            assert!(
                info.conditional,
                "{owner}.{method} marks the class conditional"
            );
        }
    }

    #[test]
    fn detects_optional_marker_as_conditional() {
        let mut w = ClassWriter::default();
        let obj = w.class("java/lang/Object");
        let this = w.class("mymod/compat/TopIntegration");
        // the marker only needs to exist as a Utf8 in the pool
        w.utf8("Lnet/minecraftforge/fml/common/Optional$Method;");
        let bytes = w.build(this, obj, &[], 0);
        let info = parse_class(&bytes).expect("parses");
        assert!(
            info.conditional,
            "@Optional marker marks the class conditional"
        );
    }

    #[test]
    fn reads_mod_client_side_only() {
        let mut w = ClassWriter::default();
        let obj = w.class("java/lang/Object");
        let this = w.class("mymod/ClientMod");
        let mod_desc = w.utf8("Lnet/minecraftforge/fml/common/Mod;");
        let client_name = w.utf8("clientSideOnly");
        let one = w.integer(1);
        let ann_attr_name = w.utf8("RuntimeVisibleAnnotations");

        // annotation body: num_annotations=1, one @Mod with clientSideOnly=Z 1
        let mut body = Vec::new();
        body.extend_from_slice(&1u16.to_be_bytes()); // num_annotations
        body.extend_from_slice(&mod_desc.to_be_bytes()); // type_index
        body.extend_from_slice(&1u16.to_be_bytes()); // num_element_value_pairs
        body.extend_from_slice(&client_name.to_be_bytes()); // element_name_index
        body.push(b'Z'); // tag
        body.extend_from_slice(&one.to_be_bytes()); // const_value_index

        let mut attrs = Vec::new();
        attrs.extend_from_slice(&ann_attr_name.to_be_bytes());
        attrs.extend_from_slice(&(body.len() as u32).to_be_bytes());
        attrs.extend_from_slice(&body);

        let bytes = w.build(this, obj, &attrs, 1);
        let info = parse_class(&bytes).expect("parses");
        assert_eq!(info.mod_sides, Some((true, false)));
    }

    #[test]
    fn reads_super_and_interfaces() {
        let bytes = fixtures::build_class_spec(&fixtures::ClassSpec {
            this: "mymod/BlockMachine",
            super_name: Some("net/minecraft/block/Block"),
            interfaces: &["net/minecraftforge/fml/common/IWorldGenerator"],
            ..Default::default()
        });
        let info = parse_class(&bytes).expect("parses");
        assert_eq!(
            info.super_name.as_deref(),
            Some("net/minecraft/block/Block")
        );
        assert_eq!(
            info.interfaces,
            vec!["net/minecraftforge/fml/common/IWorldGenerator"]
        );
        // a bare Object super is uninformative and stays None
        let bare = fixtures::build_class("mymod/Plain", &[], false, None);
        assert_eq!(parse_class(&bare).unwrap().super_name, None);
    }

    #[test]
    fn reads_acceptable_remote_versions() {
        let bytes = fixtures::build_class_spec(&fixtures::ClassSpec {
            this: "mezz/jei/JustEnoughItems",
            modid: Some("jei"),
            arv: Some("*"),
            ..Default::default()
        });
        let info = parse_class(&bytes).expect("parses");
        assert_eq!(info.mod_id.as_deref(), Some("jei"));
        assert_eq!(info.acceptable_remote_versions.as_deref(), Some("*"));
    }

    #[test]
    fn reads_class_level_dist_annotation_across_eras() {
        for (desc, konst, want) in [
            (
                "Lnet/minecraftforge/fml/relauncher/SideOnly;",
                "CLIENT",
                Dist::Client,
            ),
            ("Lcpw/mods/fml/relauncher/SideOnly;", "SERVER", Dist::Server),
            (
                "Lnet/minecraftforge/api/distmarker/OnlyIn;",
                "DEDICATED_SERVER",
                Dist::Server,
            ),
            ("Lnet/fabricmc/api/Environment;", "CLIENT", Dist::Client),
        ] {
            let bytes = fixtures::build_class_spec(&fixtures::ClassSpec {
                this: "mymod/gui/Screen",
                dist: Some((desc, konst)),
                ..Default::default()
            });
            let info = parse_class(&bytes).expect("parses");
            assert_eq!(info.dist, Some(want), "{desc}({konst})");
        }
        // no dist annotation -> None
        let bare = fixtures::build_class("mymod/Common", &[], false, None);
        assert_eq!(parse_class(&bare).unwrap().dist, None);
    }

    #[test]
    fn detects_sided_proxy_and_loading_plugin() {
        let proxied = fixtures::build_class_spec(&fixtures::ClassSpec {
            this: "mymod/Main",
            sided_proxy: true,
            ..Default::default()
        });
        assert!(parse_class(&proxied).unwrap().sided_proxy);

        let plugin = fixtures::build_class_spec(&fixtures::ClassSpec {
            this: "mymod/asm/CorePlugin",
            interfaces: &["net/minecraftforge/fml/relauncher/IFMLLoadingPlugin"],
            ..Default::default()
        });
        assert!(parse_class(&plugin).unwrap().loading_plugin);

        let plain = fixtures::build_class("mymod/Plain", &[], false, None);
        let info = parse_class(&plain).unwrap();
        assert!(!info.sided_proxy && !info.loading_plugin);
    }

    #[test]
    fn parses_pool_with_a_long_two_slot_entry() {
        // A Long consumes two pool slots; if the parser miscounts, every index
        // after it shifts and this_class / references resolve to the wrong entry.
        let mut w = ClassWriter::default();
        let obj = w.class("java/lang/Object");
        w.long(0x0123_4567_89ab_cdef);
        let this = w.class("some/Mod");
        w.class("other/Lib");
        let bytes = w.build(this, obj, &[], 0);
        let info = parse_class(&bytes).expect("parses past a long constant");
        assert_eq!(
            info.this_class, "some/Mod",
            "index alignment survived the long"
        );
        assert!(info.referenced.contains("other/Lib"));
    }

    #[test]
    fn rejects_non_class_bytes() {
        assert!(parse_class(b"PK\x03\x04not a class").is_none());
        assert!(parse_class(&[]).is_none());
        // right magic, truncated body
        assert!(parse_class(&0xCAFE_BABEu32.to_be_bytes()).is_none());
    }
}
