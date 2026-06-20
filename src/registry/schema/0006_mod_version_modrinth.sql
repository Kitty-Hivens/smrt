-- A Modrinth-sourced artifact is referenced in a build by project + version id,
-- not hosted in the local cache. Recording the version id (the project id is
-- already a mod_alias) lets the panel re-add such a mod as a real Modrinth
-- source instead of a smrt_cache one that would have no local jar to serve.
-- Nullable: cache/GitHub artifacts have no Modrinth version.
ALTER TABLE mod_version ADD COLUMN modrinth_version_id TEXT;
