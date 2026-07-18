-- Per-jar classification, keyed by content hash (side+required rework, stage
-- D). Purely derived: the harvest rewrites every scanned jar's row each run,
-- so there is no source/precious column -- same policy as mod_package. Keyed
-- by sha1 rather than mod_version because a bare coremod/library jar has no
-- mod identity (ChickenASM-class), yet the resolve report must still call it
-- a coremod.
CREATE TABLE jar_class (
  sha1         TEXT PRIMARY KEY,
  kind         TEXT NOT NULL,   -- 'mod' | 'coremod' | 'library'
  side         TEXT,            -- 'both' | 'client' | 'server'; NULL undecided
  match_policy TEXT             -- 'must_match' | 'tolerant'; NULL undecided
);

-- jar_class supersedes the per-artifact columns: 0010's side never gained a
-- reader, and 0012's match_policy moves here before gaining one. Dropped so
-- the classification has exactly one home; the next harvest repopulates.
ALTER TABLE mod_version DROP COLUMN side;
ALTER TABLE mod_version DROP COLUMN match_policy;
