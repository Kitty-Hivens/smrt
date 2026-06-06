-- Known loaders (family DAG + typical java). Open taxonomy: a target/loader_id
-- string need not appear here; this seeds what we can reason about today. Add a
-- niche loader by inserting a row + its parent edge -- no code/schema change.

INSERT INTO loader (id, display, runtime, java_major, notes) VALUES
  ('any',       'Any loader', NULL,  NULL, 'loader-agnostic artifact (tweakers, JVM agents, pure libs)'),
  ('forge',     'Forge',      'jvm', NULL, NULL),
  ('neoforge',  'NeoForge',   'jvm', NULL, NULL),
  ('fabric',    'Fabric',     'jvm', NULL, NULL),
  ('quilt',     'Quilt',      'jvm', NULL, 'Fabric-compatible'),
  ('cleanroom', 'Cleanroom',  'jvm', 25,   '1.12.2 lwjgl3 + modern-Java Forge fork');

-- family edges: child inherits parent (a child build may use parent artifacts).
INSERT INTO loader_parent (child_id, parent_id) VALUES
  ('cleanroom', 'forge'),
  ('quilt',     'fabric');
