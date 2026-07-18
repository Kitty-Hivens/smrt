-- Authored jar classification (the debug-rung escape hatch for the legacy
-- layer). jar_class rows were purely harvest-derived; an operator override for
-- a jar the classifier cannot decide (or decides with a low-confidence
-- heuristic) needs to survive re-harvest, so rows gain the standard source
-- marker: 'harvested' rows refresh each run, 'authored' rows are precious.
-- Authored classification is refused for Modrinth-identified mods -- the
-- project environment flags stay authoritative and hand-unoverridable.
ALTER TABLE jar_class ADD COLUMN source TEXT NOT NULL DEFAULT 'harvested';
