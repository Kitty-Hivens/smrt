-- Author attribution for the mod-identity index. slug + canonical_name already
-- exist on `mods` (unpopulated until the enriching harvest); this adds the
-- author so the panel can offer it as a pick-time facet. Nullable: many mods
-- carry no author metadata, and a re-harvest fills it in over time.
ALTER TABLE mods ADD COLUMN author TEXT;
