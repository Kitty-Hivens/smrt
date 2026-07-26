-- Human names for Modrinth projects the registry references but does not host.
-- A relation stores only the opaque selector (e.g. a mod's optional_dep on
-- `modrinth:xN5jnYB4` for a mod that is not mirrored); the graph view resolves it
-- through this cache so an external leaf reads as "The Aether", not the bare id.
-- Populated lazily by the modrinth-names endpoint; a harvest pass can fill it at
-- write time later. Pure display cache -- safe to drop and refill, moves no
-- compatibility fact.
CREATE TABLE IF NOT EXISTS modrinth_project_name (
    project_id TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    slug       TEXT,
    fetched_at TEXT NOT NULL
);
