-- lwjgl3ify: a 1.7.10 Forge patcher (LWJGL3 + Java 17/21 via RetroFuturaBootstrap),
-- the LWJGL3 sibling of Cleanroom's 1.12.2 fork. It runs Forge 1.7.10 artifacts
-- natively -- UniMixins, Angelica, every ordinary 1.7.10 mod -- so a pack on this
-- loader must not flag its Forge mods as loader-mismatched. Registering it with a
-- Forge parent edge is exactly what the resolver's inheritance check reads (0002
-- seeds the same shape for cleanroom -> forge); no code change, just the row.

INSERT OR IGNORE INTO loader (id, display, runtime, java_major, notes) VALUES
  ('lwjgl3ify', 'LWJGL3ify', 'jvm', 21, '1.7.10 LWJGL3 + modern-Java Forge patcher (RetroFuturaBootstrap)');

INSERT OR IGNORE INTO loader_parent (child_id, parent_id) VALUES
  ('lwjgl3ify', 'forge');
