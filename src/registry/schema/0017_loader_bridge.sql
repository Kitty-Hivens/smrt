-- Known loader bridges: a mod that carries another loader's artifacts at
-- runtime. The resolver already understands the fact -- a present mod that
-- `provides` the `loader:<name>` capability makes a foreign-loader artifact
-- carried rather than dead (#50) -- but nothing ever produced it: the
-- capability could only be entered by hand through the graph editor, so on a
-- fresh mirror every fabric mod behind a connector read as "will not load".
--
-- Open taxonomy, same shape as the loader seed: a row here is data, not code.
-- Add a bridge by inserting its Modrinth project id and the loader it carries;
-- the harvest turns that into the `provides` edge next time it reads the jar.
CREATE TABLE IF NOT EXISTS loader_bridge (
  project_id TEXT PRIMARY KEY,          -- Modrinth project of the bridge mod
  loader_id  TEXT NOT NULL REFERENCES loader(id),
  note       TEXT
);

INSERT OR IGNORE INTO loader_bridge (project_id, loader_id, note) VALUES
  ('u58R1TMW', 'fabric', 'Sinytra Connector -- runs Fabric mods on NeoForge');
