-- Incompatibility is one relation with two intensities, not two relations (#129).
--
-- The model carried `conflicts` and `breaks` as separate kinds, and every parser
-- landed on the same convention without ever writing it down: the harder
-- declaration became `conflicts`, the softer one `breaks`. Modrinth
-- `incompatible`, `mods.toml` `incompatible` and `fabric.mod.json` `breaks` are
-- all hard and all stored as `conflicts`; `mods.toml` `discouraged` and
-- `fabric.mod.json` `conflicts` are advisory and stored as `breaks`.
--
-- Everything that reported one said the opposite, because in Fabric's own
-- vocabulary `breaks` IS the hard one -- so the reporting assumed the variant
-- name meant what its namesake means, and printed the alarming word on the mild
-- declaration. Renaming the variants would have fixed the sentence and left the
-- trap: two kinds for one fact, told apart by names that argue with the
-- ecosystem they came from.
--
-- So the fact becomes one kind with a severity. `severity` is meaningful only
-- for an incompatibility, and the CHECK says exactly that rather than leaving a
-- column that is sometimes furniture.
--
-- The table is rebuilt because SQLite cannot alter a CHECK constraint, and the
-- kind vocabulary is one.

CREATE TABLE relation_new (
  id                   INTEGER PRIMARY KEY,
  from_mod_id          INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  from_mod_version_id  INTEGER REFERENCES mod_version(id) ON DELETE CASCADE,
  target_modid         TEXT NOT NULL,
  target_version_range TEXT,
  kind                 TEXT NOT NULL,
  -- 'hard'  -- the loader refuses to run the two together
  -- 'soft'  -- the author advises against it; it runs
  severity             TEXT,
  source               TEXT NOT NULL,
  confidence           INTEGER NOT NULL,
  created_at           TEXT NOT NULL,
  CHECK (kind IN ('requires','conflicts','optional_dep','provides','recommends')),
  CHECK (source IN ('harvested','jar-meta','modrinth','inferred','curator','authored')),
  CHECK (severity IS NULL OR severity IN ('hard','soft')),
  -- an incompatibility always states how hard it is; nothing else states one
  CHECK ((kind = 'conflicts') = (severity IS NOT NULL))
);

INSERT INTO relation_new
  (id, from_mod_id, from_mod_version_id, target_modid, target_version_range,
   kind, severity, source, confidence, created_at)
SELECT
  id, from_mod_id, from_mod_version_id, target_modid, target_version_range,
  CASE WHEN kind IN ('conflicts','breaks') THEN 'conflicts' ELSE kind END,
  CASE kind WHEN 'conflicts' THEN 'hard' WHEN 'breaks' THEN 'soft' ELSE NULL END,
  source, confidence, created_at
FROM relation;

DROP TABLE relation;
ALTER TABLE relation_new RENAME TO relation;

CREATE INDEX idx_rel_from ON relation(from_mod_id);
CREATE INDEX idx_rel_target ON relation(target_modid);
CREATE INDEX idx_rel_from_artifact ON relation(from_mod_version_id);

-- Severity joins the dedupe key. Without it the two rows a mod gets from
-- declaring one target both `incompatible` and `discouraged` -- which is
-- contradictory but legal, and which the old vocabulary kept apart by kind --
-- would collide, and the second would be dropped in silence by the
-- INSERT OR IGNORE the writer uses. Keeping both means a reader can take the
-- harder of the two rather than whichever happened to be written first.
CREATE UNIQUE INDEX idx_rel_dedupe
  ON relation(
    from_mod_id,
    COALESCE(from_mod_version_id, 0),
    target_modid,
    kind,
    COALESCE(severity, ''),
    source,
    COALESCE(target_version_range, '')
  );
