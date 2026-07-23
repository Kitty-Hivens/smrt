-- Mod capabilities a loader ships natively. On Forge these are separate mods a
-- pack declares and depends on; a modern-fork loader can bundle the same thing,
-- making that mod both redundant and -- once removed -- a dependency nothing in
-- the pack satisfies, which the resolver would flag as missing even though the
-- loader covers it at runtime.
--
-- Cleanroom is the case in hand: it loads mixins natively, so MixinBooter (the
-- Forge backport of exactly that) is dead weight on a Cleanroom pack, but Entity
-- Culling and Relictium still declare a hard dependency on it. This table is how
-- the resolver learns the loader answers that dependency.
--
-- Keyed to the exact loader, NOT its parent chain: Cleanroom inherits Forge's
-- artifacts, but Forge does not bundle mixins -- the capability is the fork's, not
-- something it got from its parent. Open taxonomy, same shape as the loader and
-- bridge seeds: a row is data, adding one needs no code change.
CREATE TABLE IF NOT EXISTS loader_provides (
  loader_id  TEXT NOT NULL REFERENCES loader(id),
  capability TEXT NOT NULL,          -- a dependency selector: modrinth:<id> or a bare modid
  note       TEXT,
  PRIMARY KEY (loader_id, capability)
);

-- MixinBooter, named both ways a mod can depend on it: its Modrinth project and
-- its Forge modid. A dependency declared either way is satisfied by Cleanroom.
INSERT OR IGNORE INTO loader_provides (loader_id, capability, note) VALUES
  ('cleanroom', 'modrinth:G1ckZuWK', 'MixinBooter -- Cleanroom loads mixins natively; the Forge backport is redundant'),
  ('cleanroom', 'mixinbooter',       'MixinBooter, by its Forge modid -- same native capability');
