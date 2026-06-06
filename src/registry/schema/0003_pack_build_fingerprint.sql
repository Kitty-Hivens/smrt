-- Content fingerprint per build (mirrors PackManifest.fingerprint): a stable
-- hash of what lands in an instance, independent of the pack_version label.
-- Lets the registry dedup/verify/address a build by content rather than by a
-- hand-assigned version string. Nullable: builds harvested before the manifest
-- carried a fingerprint leave it NULL.
ALTER TABLE pack_build ADD COLUMN fingerprint TEXT;
