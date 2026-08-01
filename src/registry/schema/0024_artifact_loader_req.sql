-- The loader build window an artifact declares (#164): JEI's
-- `neoforge [21.1.238,)`. Not a relation -- the loader is always present, so
-- this was never a dependency to satisfy, and dropping it on that reasoning is
-- why a pack pinned below a mod's floor published clean and died on launch.
--
-- Keyed by the artifact identity the mirror has, which is not always a hash: a
-- Modrinth pin's bytes never touch this disk, and its window is read remotely
-- before any build resolves it to a sha1. So the key is a sha1 for a jar the
-- mirror holds and `modrinth:<version_id>` for one it does not -- the same
-- selector namespacing `relation.target` uses.
--
-- A row per (artifact, loader): a jar naming two loaders declares one window
-- apiece. A single row with an empty loader is the negative -- read, declares
-- nothing -- so a jar that says nothing is not re-read on every check.
CREATE TABLE artifact_loader_req (
  artifact_key  TEXT NOT NULL,
  loader        TEXT NOT NULL DEFAULT '',
  version_range TEXT,
  read_at       TEXT NOT NULL,
  PRIMARY KEY (artifact_key, loader)
);
