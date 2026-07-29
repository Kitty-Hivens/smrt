-- What the harvest read out of a jar, whether or not it could identify it.
--
-- The unidentified-jar listing was a sha1 and a file size (#123), so working
-- through it meant downloading each jar and opening it by hand -- which is why
-- nobody did, and the bucket only grew. The harvest already opens every one of
-- these files and reads exactly what is needed to name them; it simply threw
-- that away when no alias could be derived.
--
-- Keyed by content hash like jar_class, and written for every scanned jar
-- before the identity gate, so a jar with no mod row still has a name.
CREATE TABLE jar_read (
  sha1      TEXT PRIMARY KEY,
  modid     TEXT,
  name      TEXT,
  version   TEXT,
  loaders   TEXT,  -- comma-separated, as declared
  mc        TEXT,  -- comma-separated target Minecraft versions
  filename  TEXT   -- the name it was first seen under, for a jar that declares none
);
