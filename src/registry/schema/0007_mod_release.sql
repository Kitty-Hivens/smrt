-- Version node over files (Modrinth-style): a `mod_release` groups the
-- `mod_version` artifacts (files) that share a version number. ADDITIVE -- the
-- file stays `mod_version` and remains the build spine (`pack_build_mod` keeps
-- pointing at it); a `release_id` column links each file up to its release.
--
-- channel (release/beta/dev/unknown) is a release-level fact the harvest cannot
-- derive yet, so it defaults to 'unknown'; the authored layer sets it. The
-- (mod_id, version_number, channel) key lets one number coexist across channels
-- (a 'dev' 1.0 alongside a 'release' 1.0) while still grouping a mod's files that
-- share a number. Backfill collapses existing files by (mod_id, version) into one
-- 'unknown' release each; all current content is forge-1.12.2, so it is ~1:1.

CREATE TABLE mod_release (
  id             INTEGER PRIMARY KEY,
  mod_id         INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  version_number TEXT NOT NULL,
  channel        TEXT NOT NULL DEFAULT 'unknown',
  source         TEXT NOT NULL DEFAULT 'harvested',
  confidence     INTEGER NOT NULL DEFAULT 10,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL,
  CHECK (channel IN ('release','beta','dev','unknown')),
  CHECK (source IN ('harvested','jar-meta','modrinth','inferred','curator','authored'))
);
CREATE INDEX idx_mod_release_mod ON mod_release(mod_id);
CREATE UNIQUE INDEX idx_mod_release_key ON mod_release(mod_id, version_number, channel);

-- the file's link up to its release. NULL = not yet grouped (a harvested file
-- gets one immediately; an authored regroup can move it). ON DELETE SET NULL so
-- dropping a release orphans its files rather than deleting artifacts.
ALTER TABLE mod_version ADD COLUMN release_id INTEGER REFERENCES mod_release(id) ON DELETE SET NULL;
CREATE INDEX idx_mv_release ON mod_version(release_id);

-- backfill: one 'unknown' release per (mod_id, version) over existing files,
-- then point each file at it.
INSERT INTO mod_release (mod_id, version_number, channel, source, confidence, created_at, updated_at)
  SELECT mod_id, version, 'unknown', 'harvested', 10, min(created_at), max(updated_at)
  FROM mod_version
  GROUP BY mod_id, version;
UPDATE mod_version SET release_id = (
  SELECT r.id FROM mod_release r
  WHERE r.mod_id = mod_version.mod_id
    AND r.version_number = mod_version.version
    AND r.channel = 'unknown'
);
