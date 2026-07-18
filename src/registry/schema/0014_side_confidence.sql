-- Confidence of the derived side (stage I hardening): 'high' -- an explicit
-- marker decided it (@Mod side flags, fabric env/entrypoints, content
-- registration, paired flags); 'low' -- the blanket client-surface heuristic,
-- which can misread a client-heavy library (bspkrsCore-class) as client. The
-- client invariant refuses a build only over a high-confidence verdict; a
-- declared hard edge outweighs a low one. Derived, rewritten each harvest.
ALTER TABLE jar_class ADD COLUMN side_confidence TEXT;
