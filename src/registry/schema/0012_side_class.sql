-- Side/match-policy inputs for the presence classification (side+required
-- rework, stage B). Additive; all three columns are derived facts the harvest
-- rebuilds -- NULL means "not derived yet", never "decided empty".

-- The Modrinth project's declared environment flags, verbatim
-- ('required' | 'optional' | 'unsupported' | whatever upstream ships).
-- Project-level facts, so they live on mods, not on the artifact. Written by
-- the harvest enrichment for a Modrinth-identified mod (the modrinth layer,
-- refreshed each run, precious rows skipped); the classifier maps them onto
-- (side, match policy) with priority over the bytecode derivation.
ALTER TABLE mods ADD COLUMN client_env TEXT;
ALTER TABLE mods ADD COLUMN server_env TEXT;

-- The jar's server-match policy derived by the bytecode classifier, beside
-- `side` (0010): 'must_match' | 'tolerant'. A per-artifact fact -- two builds
-- of one mod can differ (a 1.12 jar with acceptableRemoteVersions="*" vs an
-- older one without). Written like `side`: COALESCE on refresh so an undecided
-- re-derivation never erases a known value, precious rows skipped.
ALTER TABLE mod_version ADD COLUMN match_policy TEXT;
