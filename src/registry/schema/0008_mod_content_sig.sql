-- Content signature: a hash of a jar's contents EXCLUDING version-bearing
-- metadata (mcmod.info, META-INF/mods.toml, fabric.mod.json, MANIFEST). Two jars
-- with the same signature are the same real build with different declared
-- versions -- the SmartyCraft case, where only the mcmod.info version string is
-- bumped (1.3.5 -> "1.3.6") so the bytes (and sha1) differ but the mod does not.
-- Harvest groups such files into one release instead of fragmenting on the faked
-- version. NULL for jars harvested before this column, or not readable as a zip.
ALTER TABLE mod_version ADD COLUMN content_sig TEXT;
CREATE INDEX idx_mv_content_sig ON mod_version(mod_id, content_sig);
