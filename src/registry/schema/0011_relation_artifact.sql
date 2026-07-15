-- Relations belong to the artifact that declares them, not to the mod (#48).
--
-- The model was asymmetric: the target carried `target_version_range` while the
-- source was a bare `from_mod_id`. The harvest already derives every edge from a
-- specific jar and then threw that away on write, so two harvested versions of a
-- mod unioned their dependency sets onto one node -- a registry holding JEI for
-- 1.12.2 and 1.19.2 attributed the 1.19 jar's deps to the 1.12 one. A pack ships
-- one artifact, so the resolver and the graph were both reasoning over facts the
-- shipped file never declared.
--
-- `from_mod_version_id` is nullable on purpose, and the two states are different
-- claims rather than a convenience:
--   NOT NULL -- this edge was derived from (and is true of) this one artifact.
--   NULL     -- a statement about the mod as a whole. That is what a human means
--              when authoring "X conflicts with Y", and it is also the honest
--              record for a derived row whose artifact we can no longer name.
-- `source` tells the two apart (authored/curator vs the derived kinds).
ALTER TABLE relation ADD COLUMN from_mod_version_id INTEGER REFERENCES mod_version(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_rel_from_artifact ON relation(from_mod_version_id);

-- Backfill without inventing facts. Where a mod has exactly one artifact there is
-- no doubt which jar a derived edge came from, so attach it. Where a mod has
-- several we genuinely no longer know, so the row stays NULL: that reads as a
-- mod-level fact, which is precisely today's behaviour, so nothing regresses --
-- and the next harvest replaces it with a properly scoped row. Authored and
-- curator rows are left alone: they were asserted about the mod, and NULL says so.
UPDATE relation
   SET from_mod_version_id = (
         SELECT mv.id FROM mod_version mv WHERE mv.mod_id = relation.from_mod_id
       )
 WHERE source IN ('harvested', 'jar-meta', 'inferred', 'modrinth')
   AND (SELECT count(*) FROM mod_version mv WHERE mv.mod_id = relation.from_mod_id) = 1;

-- The dedupe key gains the artifact: the same edge may now legitimately exist once
-- per artifact of a mod (a forge build and a fabric build of one version really do
-- declare different things). COALESCE keeps the mod-level rows deduped among
-- themselves, since NULLs are all distinct to a UNIQUE index.
DROP INDEX IF EXISTS idx_rel_dedupe;
CREATE UNIQUE INDEX idx_rel_dedupe
  ON relation(
    from_mod_id,
    COALESCE(from_mod_version_id, 0),
    target_modid,
    kind,
    source,
    COALESCE(target_version_range, '')
  );
