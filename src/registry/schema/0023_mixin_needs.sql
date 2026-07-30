-- What a jar's required mixins must be able to resolve, and what each artifact
-- provides to resolve it with (#145).
--
-- A mixin config that says `"required": true` is a promise: if what it patches
-- is not there, the loader throws during init and the crash names whichever mod
-- first reached the missing class rather than the one that asked for it. Twice
-- in one day a published pack met exactly this. Neither case was visible in the
-- metadata -- both mods declare an open lower bound on the host, which every
-- version satisfies.
--
-- Answering it at build time means asking "does the pack's copy of X still have
-- class C?", so both halves are stored here.

-- The needs: small, tens of rows per artifact, and only for `required` configs.
-- Scoped to the artifact rather than the mod because two builds of one mod patch
-- different things -- which is the whole reason the question arises.
CREATE TABLE mixin_need (
  mod_version_id INTEGER NOT NULL REFERENCES mod_version(id) ON DELETE CASCADE,
  -- the config that declares it, so a finding can point at the declaration
  config         TEXT NOT NULL,
  -- binary name (`net/caffeinemc/mods/sodium/client/gui/SodiumGameOptions`)
  needed         TEXT NOT NULL,
  PRIMARY KEY (mod_version_id, config, needed)
);
CREATE INDEX idx_mixin_need_needed ON mixin_need(needed);

-- What an artifact provides, as a sorted run of 8-byte big-endian hashes of its
-- class names -- membership by binary search, nothing else asked of it.
--
-- Hashes rather than names because the names are the bulk: Sodium carries 871
-- classes, which is some 43 KB of text per artifact and hundreds of megabytes
-- across a registry, against 7 KB here. A 64-bit hash makes a false "present"
-- vanishingly unlikely and a false "missing" impossible, and a false "present"
-- only means one finding is not raised.
--
-- NULL for an artifact the mirror has never had the bytes of, which reads as
-- "cannot answer" and never as "empty".
ALTER TABLE mod_version ADD COLUMN class_digest BLOB;
