# Side / required-ness / dependency derivation: audit and baseline

Stage A of the side+required rework. Read-only snapshot of how the facts flow
today (2026-07-18, HEAD 569bcc4), what the real data shows, and the measuring
base the later stages are judged against. No code was changed in this stage.

Companion artifacts:

- `testdata/side-labels.toml` -- hand-labeled ground truth for 51 corpus jars
  (side / match policy / kind / presence category).
- `testdata/corpus/fetch.py` -- re-fetches the corpus (every jar of every
  published pack, Modrinth version + project objects, and a prod-faithful
  storage replica for `smrt-pack registry harvest`). Jars are content-addressed
  and verified by sha1; the corpus itself stays out of the repo (~370 MB).
- `testdata/corpus/baseline.py` -- faithful port of `dependency_fill_plan` +
  `apply_requires` + `derive_required` over the harvested replica registry.
- `testdata/corpus/baseline.json` -- the recorded baseline (per-pack required
  sets with driver edges, plus the hard-edges-into-client-mods evidence).

## 1. Fact trace: producer -> store -> consumer

### side

| Step | Where | What happens |
| --- | --- | --- |
| derive | `authoring/classfile.rs` `parse_class` | reads `@Mod(clientSideOnly/serverSideOnly)` element values (Forge 1.7-1.12, both `net.minecraftforge` and `cpw.mods` spellings) |
| derive | `authoring/bytecode.rs` `derive_side` / `fabric_side_from_json` | folds `@Mod` flags across classes; falls back to `fabric.mod.json` `environment`. `@Mod` wins over fabric env (`aggregate`). No other signal exists |
| carry | `authoring/harvest.rs` `JarSeed.side` | only for jars whose bytes are locally cached; a Modrinth-only mod is never scanned |
| store | `registry/upsert.rs` `set_mod_version_side` -> `mod_version.side` (migration 0010) | COALESCE (an undecided re-derivation never erases), precious rows skipped |
| consume | -- | **nothing reads the column.** Confirmed by grep: the only non-writer mentions are the `sides_derived` report counters |

`mods.toml` / `neoforge.mods.toml` carry no mod-level side, and the modern
loaders dropped the `@Mod` side flags, so every modern-era jar stays `NULL`
(confirmed on the corpus: all five NeoForge cache jars -- FTBLibrary, FTBTeams,
FTBQuests, Configured, open-smrt-network-1.21.1 -- have no side). Modrinth
project env flags (`client_side` / `server_side`) are not requested at all:
`authoring/modrinth.rs` `Project` deserializes only `{id, slug, title, team}`.

### dependency edges (requires / optional_dep / recommends / conflicts / breaks / provides)

| Producer | Source tag | Notes |
| --- | --- | --- |
| Modrinth `version.dependencies` (harvest batch lookup) | `modrinth` | authoritative when non-empty: suppresses jar declaration AND bytecode for that jar (`harvest.rs` `is_modrinth`). Targets in the `modrinth:<project_id>` namespace; pinned `version_id` rides in the range slot. `embedded` yields no edge |
| `mcmod.info` (`requiredMods` else `dependencies`, filtered) | `jar-meta` | `mcmod_hard_deps` + `filter_deps` + `clean_dep_token`; pseudo-deps (forge/fml/mcp/...) dropped |
| `mods.toml` / `neoforge.mods.toml` / `fabric.mod.json` | `jar-meta` | typed + version-ranged (`modmeta.rs`); `type`/`mandatory` mapped to RelKind; platform modids dropped. **Not read:** `displayTest`, per-dependency `side` |
| bytecode package references | `inferred` | class-granularity: one unconditional referencing class makes the prefix hard; conditional = `isModLoaded` Methodref / `@Optional` / plugin-marker Utf8 in the pool. Prefix -> owner via `mod_package`; multi-owner prefixes dropped; `INTEGRATION_HOSTS` (item viewers/probes) downgraded to optional at write (bd3b4ce) |
| pack `display.incompatible_with` | `curator` | mod-level mutual conflicts |
| operator CLI / panel | `authored` | precious |
| fabric `recommends`/`suggests` -> `OptionalDep`; **nothing ever emits `Recommends`** | -- | `RelKind::Recommends` exists in the vocab, is parsed, and is skipped by the resolver; no producer, no UI |

Consumers: `resolve.rs` (`resolve_pack`, `dependency_fill_plan`, `pack_graph`)
read edges at artifact granularity ordered by confidence, first edge per target
wins; `depfill.rs` `fill_dependencies` (on config save) auto-adds missing hard
deps *from Modrinth only* and rewrites every mod's `display.requires`;
`queries.rs` graph views.

### required / default_enabled

- `domain/pack.rs` `DeclaredMod`: **no manual required flag** (the doc comment
  is the law); only `default_enabled` is authored.
- `authoring/build.rs` `derive_required`: BFS over hard `display.requires`
  edges seeded from *the dependencies of* default-enabled mods; a top-level mod
  nothing depends on is never required. Emitted on the wire `ModEntry.required`.
- `resolve.rs` treats `default_enabled` as the only install-default signal;
  conflicts with an opted-out side are advisory.

## 2. Signal inventory vs the D.1 matrix

Already extracted today: `@Mod` `clientSideOnly`/`serverSideOnly`/`modid`
element values; `isModLoaded`-family Methodrefs (Forge 1.7-1.12, modern Forge,
NeoForge, Fabric, Quilt); `@Optional.*` + integration-plugin marker Utf8s;
class ownership + referenced types (constant pool + field/method descriptors);
`fabric.mod.json` `environment`; loader marker files.

Missing for D.1: `@Mod(acceptableRemoteVersions)` element value;
`@SideOnly`/`@OnlyIn`/`@Environment` annotation descriptors (class-level);
content-registry signals (`GameRegistry`, `RegistryEvent$Register`,
`DeferredRegister`, `net/minecraft/block|item|entity` supertypes);
client-package blanket analysis (`net/minecraft/client/**`); coremod markers
(`FMLCorePlugin` manifest attribute, `IFMLLoadingPlugin` implementations,
`*.mixins.json` without a mod identity); `mods.toml` `displayTest` and
per-dependency `side`; fabric client entrypoints.

## 3. Corpus

Two published packs, 141 distinct jars (fetched 2026-07-18):

| pack | mc / loader | mods | smrt_cache | modrinth |
| --- | --- | --- | --- | --- |
| Create | 1.21.1 neoforge | 50 | 5 | 45 |
| Industrial | 1.12.2 forge | 91 | 60 | 31 |

Modrinth knows 77 of 141 jars by sha1 (76 declared Modrinth sources + the
CodeChickenLib cache jar). 97 project objects fetched (incl. dependency
targets); env-flag spread: req/req 54, req/uns 14 (client-only), opt/opt 14,
req/opt 8, uns/req 4 (server-only), opt/req 3.

Labels: 51 jars in `testdata/side-labels.toml` -- required 14,
optional_client 17, optional_both 17, optional_server 2, coremod 1. The two
published packs simply do not contain more server-only or coremod-only jars;
stage D tops those two categories up with fetched reference jars (picked by
Modrinth env `unsupported/required` for server-side; known 1.12-era coremods)
plus synthetic class fixtures per D.1 matrix row.

## 4. Baseline metrics

Pipeline as run: `smrt-pack registry harvest` against the prod-faithful
replica (cache jars scanned, Modrinth jars identity-only -- same as prod), then
the `baseline.py` port of the post-569bcc4 save+build path.

Harvest: 141 jars scanned, 1 no-identity (ChickenASM: no @Mod, no mcmod.info,
no Modrinth match), 45 inferred hard edges, 12 inferred optional, 55 modrinth
edges, 8 typed declared edges, 59 sides derived.

Side coverage: 59/141 artifacts (42%). All 59 come from `@Mod` flags on 1.12.2
cache jars; 0 modern jars sided; 0 Modrinth-only jars scanned.

Required-set baseline ("next build after a config save", i.e. what depfill +
derive_required produce from today's registry):

| pack | published required (legacy manual flags) | required on next build | flips |
| --- | --- | --- | --- |
| Create | 43 / 50 | 11 / 50 | 32 |
| Industrial | 52 / 91 | 20 / 90 placed | 40 |

Both published manifests predate afb49f9 (they carry hand-set required flags
and empty `display.requires`), so the published state and the next-build state
differ massively. The full per-mod table with driver edges is
`testdata/corpus/baseline.json`.

## 5. Confirmed defects (each verified on the corpus)

**F1 -- top-level content goes optional (symptom 1).** On next build the Create
pack keeps required = {Create, Architectury, ClothConfig, FarmersDelight, FTB
libs, Sophisticated libs, Sable, Mechanicals, DragonsPlus} -- every leaf
content mod (ArsNouveau, FTBQuests, every Create-* addon, StorageDrawers,
IronChest, ...) becomes toggleable. A player who disables ArsNouveau cannot
join a server running it. Same for Industrial (Quark, RailCraft-as-leaf after
F3 is fixed, CustomNPCs, ...).

**F2 -- a client mod gets locked required through an inferred hard edge
(symptom 2).** `Chisel -> ctm [inferred]`: ConnectedTexturesMod (side=client
in the registry) flips published=optional -> next-build=required. Violates the
project rule that client mods are never force-installed.

**F3 -- bundled foreign APIs forge false ownership.** RailCraft's jar bundles
`ic2/api`, `thaumcraft/api`, `atomicstryker/dynamiclights`. The genuine IC2 is
a Modrinth source, so its bytes are never scanned and it owns no packages;
RailCraft becomes the *sole* owner of `ic2/api`, and every IC2-API consumer
(AE2, AdvancedMachines, AdvancedSolarPanels, BCFuelsForIC2, EnergyControl,
GravitationSuite) gets an inferred hard edge -> RailCraft. Verified: AE2's
pool references `ic2/api` and contains zero references to `mods/railcraft`.
The multi-owner drop only protects when both owners are scanned; the
scanned/unscanned asymmetry defeats it.

**F4 -- integration references read as unconditional hard (class
granularity).** `Forestry -> InventoryTweaks [inferred]`:
`forestry/core/gui/ContainerForestry.class` references only InvTweaks API
annotations -- a dormant integration -- with no `isModLoaded`/`@Optional`
marker in that class, so it grades hard. Same mechanism behind
`AppliedEnergistics2 -> InventoryTweaks` and `RailCraft -> Forestry` (AE2 and
RailCraft gate integrations through their own registries, invisible to the
marker allowlist).

**F5 -- the side signal is stored but dead, and structurally absent for modern
jars.** `mod_version.side` has no reader. The only producers (`@Mod` flags,
fabric env) cannot side a NeoForge/modern-Forge jar (all five NeoForge cache
jars are NULL), and Modrinth-sourced jars are never scanned at all. Modrinth
env flags -- the priority-1 source per the plan -- are not even requested.

**Data note -- Modrinth env flags are authored and sometimes wrong.** The
`create` project declares client=optional, server=required; a vanilla client
cannot actually join a Create server. The plan's priority (Modrinth over
bytecode) therefore needs the E.3 disagreement advisory, not silent trust --
Create is the flagship case the advisory must surface.

## 6. Stage I: end-to-end acceptance (2026-07-18, post-rework)

Pipeline exercised on the replica exactly as production runs it: `registry
harvest` (fresh, migrations 12-14 applied), `depfill` per pack (the same pass
the panel runs on save; it pulled create_oxidized + collective for Create and
malilib + theimpossiblelibrary for Industrial, and recorded 29 + 60 hard
edges), then `build`.

Required-set outcome, against the stage-A baseline ("what depfill +
derive_required would have produced before the rework") and the legacy
published manifests (hand-set flags):

| pack | legacy published | pre-rework baseline | now | explanation |
| --- | --- | --- | --- | --- |
| Create | 43/50 | 11/50 | 39/52 | every content mod + addon required via must_match or the graph; client mods (ChatHeads, WorldEditCUI, Configured) and tolerant Both mods (Jade, REI, Xaeros, libs) toggleable; the 2 depfill-pulled mods unclassified until the next harvest |
| Industrial | 52/91 | 20/90 | 45/93 | all 1.12 content mods required via bytecode content signals; 20 mods badged optional_client; server-side mods (Born In A Barn, Ksyxis, hit-indication) shipped default-off; ChickenASM presence=coremod, toggleable |

Both plan symptoms verified closed on the real packs:

- symptom 1: ArsNouveau, FTBQuests, every Create-* addon, Quark, RailCraft,
  CustomNPCs, ... are required with zero hand-set flags;
- symptom 2: ConnectedTexturesMod flipped baseline-required -> optional_client
  (the inferred Chisel edge downgraded by the guard); WorldEditCUI, stuck
  required=true in the legacy manifest, is optional_client now.

The poisoned-data invariant test ran live: an authored hard edge onto `ctm`
made the Industrial build refuse with "client-side mod ConnectedTexturesMod.jar
would be locked required (hard-required by AppliedEnergistics2.jar)".

One hardening landed during acceptance: the blanket client-surface heuristic
had read bspkrsCore (a client-heavy library TreeCapitator hard-requires on both
sides) as client, and the invariant blocked a legitimate build. Side verdicts
now carry a confidence grade (`jar_class.side_confidence`, migration 0014):
explicit markers and Modrinth flags are `high` and the invariant stands;
the surface heuristic is `low`, and a declared hard edge outweighs it -- the
mod locks with a warning and the resolve report shows the pair. The corpus
ratios confirmed no threshold can separate that shape locally (bspkrsCore's
client-surface ratio, 56%, exceeds genuinely-client WailaHarvestability's 50%).

Corpus grading after the rework (tests/corpus_classify.rs over 57 labels):
cascade 52/57, bytecode-only population 22/24, zero must_match verdicts on
client-labeled jars. The five persistent misses are documented upstream-flag
quirks (Create declares client=optional; Architectury/MixinBooter declare
required/required for what are standalone-tolerant libraries -- the E-layer
advisory surfaces exactly these) plus two safe unclassified degradations
(Schematica, autoreglib's Block-base false content signal).

Harvest cost: the per-jar parse (the full single-pass reader) measured 75-97
ms/jar over the 147-jar corpus both before and after the classifier rework
(debug profile, three runs each, identical ownership checksums) -- the v2
signals stay constant-pool-bounded and the scan remains dominated by Modrinth
round-trips. Instruments: `examples/parse_bench.rs`, `examples/evidence_dump.rs`.

Left for the curator (the unclassified list on the replica registry --
match policy undecided, shipped toggleable): BCFuelsForIC2, FTBLibrary, Hats,
JEIBees, LunatriusCore, NBTEdit, ProjectRed-Integration, Schematica, WanionLib.
Each would firm up from an authored classification or better upstream flags;
none can be locked or force anything meanwhile.

## 7. Follow-up sweep (2026-07-18, same day): edge quality + authored classification

The deferred edge-quality fixes landed, plus the authored escape hatch the
unclassified list needed.

**Edge quality.** Three mechanisms, all corpus-verified:

- bundled foreign APIs no longer confer ownership: a jar owning `<root>/api`
  with no sibling package under the same root is bundling, not owning (the
  sibling test separates RailCraft's bundled ic2/api + thaumcraft/api from
  every legitimate API owner exactly -- checked across all ten /api prefixes
  in the registry);
- integration-package classes (`integration`/`compat`/`plugins`/...) grade as
  conditional when they are a minority of the jar -- embedded compat modules
  behind a mod's own plugin manager -- while a jar living entirely under such
  a namespace (ProjectRed-Integration) keeps its real hard deps;
- InventoryTweaks joined the integration-host list (containers advertise to it
  through API annotations and helper signatures);
- a jar with modern-manifest declared deps is on the manifest tier of the
  cascade: bytecode inference is suppressed for it, and on a dual-metadata jar
  the manifest edge outranks the mcmod.info one (written first; rank 55 vs 50).

Result on the replica: 11 of the 12 false inferred hard edges are gone
(45 -> 34 inferred requires). The survivor, RailCraft -> forestry, comes from
unconditional forestry-API references in RailCraft's regular item code
(ItemFilterBeeGenome), outside any integration namespace -- the honest limit of
class granularity; an authored optional_dep suppresses it through the existing
per-target precedence, which is the designed escape hatch.

**Authored classification** (migration 0015). `jar_class` rows gain the
standard source marker: harvest refreshes `harvested` rows and never touches
`authored` ones. The override is debug-gated (compat-affecting), audited,
stored with high side-confidence, and REFUSED for a Modrinth-identified mod --
the project environment flags stay authoritative and are fixed upstream, not
overridden. Surfaces: `PUT /v1/registry/files/{sha1}/class` and
`smrt-pack registry classify`.

**Final replica state.** Both packs build with zero warnings; a repeated
identical build yields an identical fingerprint. Create 40/52 required,
presence on every entry. Industrial 47/93: BCFuelsForIC2 and
ProjectRed-Integration lock via their authored must_match, InventoryTweaks is
finally toggleable (its false hard edges are gone), ChickenASM reads coremod,
the server-side trio ships opted out. The one entry without a presence badge is
lunary-mod: a curator-disabled must_match mod fits none of the five classes,
and no badge is more honest than a wrong one. Corpus bars unchanged (52/57
cascade, 22/24 bytecode-only, zero client-label must_match).

**Production runbook** (after this lands and a harvest has run): re-apply the
authored layer, then save each pack config in the panel (or run depfill) and
rebuild. Classifications, by cached sha1:

```
smrt-pack registry classify --sha1 9ae9811f387a960381e5c82fae2a7f79d0480a27 --side both   --policy tolerant    # bspkrsCore
smrt-pack registry classify --sha1 dd06d6d5a906212b5800082e7a0f6afb4fae0386 --side both   --policy must_match  # BCFuelsForIC2
smrt-pack registry classify --sha1 10a0674a76a6f5d736ee43d4179512f22fce5d03 --side both   --policy tolerant    # FTBLibrary
smrt-pack registry classify --sha1 5a2c293aec393901d3cfff52eb7a3b855999cba8 --side both   --policy tolerant    # Hats
smrt-pack registry classify --sha1 4be9841d7d97e4c0d824e2fb363f43b5b35c2f52 --side client --policy tolerant    # JEIBees
smrt-pack registry classify --sha1 f70b39168f2a02d47792cf29bef432e20d0bcd57 --side both   --policy tolerant    # LunatriusCore
smrt-pack registry classify --sha1 dbed8f269c40aecab5cebb767cef1a9815b68ce2 --side both   --policy tolerant    # NBTEdit
smrt-pack registry classify --sha1 f05cda479b14d41e02120295b4bcbb0ab25738d2 --side both   --policy must_match  # ProjectRed-Integration
smrt-pack registry classify --sha1 cc8c851bc47ecf314c3974e3b90acfa1a62105ac --side client --policy tolerant    # Schematica
smrt-pack registry classify --sha1 8518861bf46a9ec99d89f2b138ed0790fb858071 --side both   --policy tolerant    # WanionLib
```

Plus one authored edge through the panel's graph editor (or
`POST /v1/registry/relations`): RailCraft `optional_dep` -> `forestry`, which
suppresses the surviving inferred hard edge. The Modrinth env-flag
disagreements (Create declares client=optional; Architectury and MixinBooter
declare required/required) stay advisories by design: those flags are fixed
upstream, and the packs' outcomes are already correct through the graph.

## 8. Notes carried into stages B-I

- `RelKind::Recommends` needs only a producer decision + panel surfacing; the
  vocab, parser, and resolver skip-path already exist.
- The `INTEGRATION_HOSTS` downgrade (bd3b4ce) is the same shape as the F.2
  client-edge guard: precedent for "inferred hard, demoted before BFS".
- `dependency_fill_plan` + `derive_required` and this audit's `baseline.py`
  must stay in lockstep; when F lands, the diff against
  `testdata/corpus/baseline.json` is the acceptance artifact.
- Harvest wall-clock on this machine over the replica (141 jars, warm cache):
  the scan is dominated by Modrinth round-trips; the local jar parse is well
  under a second per jar. Stage I re-measures after the D classifier lands.
