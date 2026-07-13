-- Bytecode-derived dependency graph (#40). Two additive pieces the harvest fills
-- from each jar's `.class` entries, independent of what the author declared.

-- Package -> owning mod index. A jar defines classes under one or more package
-- prefixes (appeng/core, appeng/api, ...); a reference from another jar to a
-- prefix owned here becomes a dependency edge. Rebuilt wholesale each harvest
-- (purely derived), so no source/precious column. A prefix owned by more than
-- one mod is ambiguous (a shaded library) and the resolver drops it.
CREATE TABLE mod_package (
  mod_id INTEGER NOT NULL REFERENCES mods(id) ON DELETE CASCADE,
  prefix TEXT NOT NULL,
  PRIMARY KEY (mod_id, prefix)
);
CREATE INDEX idx_mod_package_prefix ON mod_package(prefix);

-- A jar's runtime side, derived from the `@Mod(clientSideOnly/serverSideOnly)`
-- annotation (Forge) or `fabric.mod.json` `environment` (modern): 'both' /
-- 'client' / 'server'. NULL = not derived (undecided -> treat as both). A hint
-- for optional-ness, not an authored fact; harvest refreshes it, the precious
-- layer is left untouched by the writer.
ALTER TABLE mod_version ADD COLUMN side TEXT;
