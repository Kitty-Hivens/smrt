-- mod-identity registry. Phase 1 populates only the harvested subset; the
-- authored layer (source='authored'/'curator') lands in Phase 2, but the
-- source/confidence columns + the never-clobber rule are designed in now.

CREATE TABLE registry_meta (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
);

-- loader taxonomy: what we KNOW about a loader (family parents, java). A loader
-- id used as a mod_version_target.target or pack_build.loader_id need NOT have a
-- row here (open taxonomy); this table only carries the family DAG + runtime facts.
CREATE TABLE loader (
  id          TEXT PRIMARY KEY,
  display     TEXT NOT NULL,
  runtime     TEXT,
  java_major  INTEGER,
  notes       TEXT
);

-- child INHERITS-FROM parent (cleanroom -> forge, quilt -> fabric): a child
-- build may use a parent-targeted artifact. One-directional.
CREATE TABLE loader_parent (
  child_id  TEXT NOT NULL REFERENCES loader(id) ON DELETE CASCADE,
  parent_id TEXT NOT NULL REFERENCES loader(id) ON DELETE CASCADE,
  PRIMARY KEY (child_id, parent_id)
);
CREATE INDEX idx_loader_parent_parent ON loader_parent(parent_id);

-- logical mod identity (surrogate key; the external keys live in mod_alias).
CREATE TABLE mods (
  id             INTEGER PRIMARY KEY,
  slug           TEXT,
  canonical_name TEXT,
  source         TEXT NOT NULL DEFAULT 'harvested',
  confidence     INTEGER NOT NULL DEFAULT 10,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL,
  CHECK (source IN ('harvested','jar-meta','modrinth','inferred','curator','authored'))
);

-- external keys: modid / modrinth project_id / future curseforge id.
CREATE TABLE mod_alias (
  mod_id        INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  source        TEXT NOT NULL,
  external_key  TEXT NOT NULL,
  PRIMARY KEY (source, external_key)
);
CREATE INDEX idx_mod_alias_mod ON mod_alias(mod_id);

-- concrete artifact (a jar). sha1 is the ONLY identity: the same bytes are one
-- artifact, different bytes are different artifacts -- even when they share a
-- version label (a rebuild, or two jars both lacking version metadata). There is
-- deliberately no (mod_id, version) uniqueness; that would crash a harvest of two
-- such jars. Compatibility targets live in mod_version_target (a jar can suit
-- several loaders), not a column, since one sha1 can't be several rows.
CREATE TABLE mod_version (
  id           INTEGER PRIMARY KEY,
  mod_id       INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  version      TEXT NOT NULL,
  sha1         TEXT NOT NULL,
  size_bytes   INTEGER NOT NULL,
  filename     TEXT,
  mc_versions  TEXT,
  source       TEXT NOT NULL DEFAULT 'harvested',
  confidence   INTEGER NOT NULL DEFAULT 10,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  CHECK (source IN ('harvested','jar-meta','modrinth','inferred','curator','authored'))
);
-- sha1 is a content address: the same bytes cannot be two artifacts.
CREATE UNIQUE INDEX idx_mv_sha1 ON mod_version(sha1);
CREATE INDEX idx_mv_mod ON mod_version(mod_id);

-- a mod_version's compatibility targets: each a loader id OR 'any'. A jar
-- published for several loaders (Modrinth lists them as a set) gets one row per
-- target; a loader-agnostic tweaker gets a single 'any' row. target is plain
-- TEXT, NOT a hard FK, so 'any' and uncatalogued loaders store cleanly.
-- INVARIANT: every mod_version has >= 1 row here (harvest falls back to 'any').
-- Q4 (eligible_for_loader) inner-joins this table, so a target-less artifact
-- would be invisible to eligibility -- any future writer must keep the fallback.
CREATE TABLE mod_version_target (
  mod_version_id INTEGER NOT NULL REFERENCES mod_version(id) ON DELETE CASCADE,
  target         TEXT NOT NULL,
  PRIMARY KEY (mod_version_id, target)
);
CREATE INDEX idx_mvt_target ON mod_version_target(target);

-- packs + builds. Loader is per-build (migration model A). loader_id is a plain
-- string (open taxonomy), advisory against the loader table.
CREATE TABLE pack (
  id         TEXT PRIMARY KEY,
  provenance TEXT NOT NULL DEFAULT 'hivens',
  source     TEXT NOT NULL DEFAULT 'harvested',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (provenance IN ('sc','hivens'))
);

CREATE TABLE pack_build (
  id             INTEGER PRIMARY KEY,
  pack_id        TEXT NOT NULL REFERENCES pack(id) ON DELETE CASCADE,
  pack_version   TEXT NOT NULL,
  mc_version     TEXT NOT NULL,
  loader_id      TEXT,
  loader_version TEXT,
  java_major     INTEGER,
  is_latest      INTEGER NOT NULL DEFAULT 0,
  source         TEXT NOT NULL DEFAULT 'harvested',
  created_at     TEXT NOT NULL,
  UNIQUE (pack_id, pack_version)
);
CREATE INDEX idx_build_pack ON pack_build(pack_id);

CREATE TABLE pack_build_mod (
  build_id        INTEGER NOT NULL REFERENCES pack_build(id) ON DELETE CASCADE,
  mod_version_id  INTEGER NOT NULL REFERENCES mod_version(id) ON DELETE CASCADE,
  filename        TEXT NOT NULL,
  required        INTEGER NOT NULL DEFAULT 1,
  default_enabled INTEGER NOT NULL DEFAULT 1,
  source          TEXT NOT NULL DEFAULT 'harvested',
  PRIMARY KEY (build_id, mod_version_id)
);
CREATE INDEX idx_pbm_mv ON pack_build_mod(mod_version_id);

-- sourced relations (open-world; transitive closure computed at resolve time,
-- never stored). target is a SELECTOR (modid + optional version range), NOT a
-- hard FK: the target mod may not be harvested yet.
CREATE TABLE relation (
  id                   INTEGER PRIMARY KEY,
  from_mod_id          INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  target_modid         TEXT NOT NULL,
  target_version_range TEXT,
  kind                 TEXT NOT NULL,
  source               TEXT NOT NULL,
  confidence           INTEGER NOT NULL,
  created_at           TEXT NOT NULL,
  CHECK (kind IN ('requires','conflicts','optional_dep','provides','recommends','breaks')),
  CHECK (source IN ('harvested','jar-meta','modrinth','inferred','curator','authored'))
);
CREATE INDEX idx_rel_from ON relation(from_mod_id);
CREATE INDEX idx_rel_target ON relation(target_modid);
CREATE UNIQUE INDEX idx_rel_dedupe
  ON relation(from_mod_id, target_modid, kind, source, COALESCE(target_version_range, ''));
