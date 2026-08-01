//! Reading one named entry out of a zip that lives on someone else's server.
//!
//! A mod's declared loader window sits in a 2 kB file inside a jar the mirror
//! does not hold: 92 of the 97 mods in this deployment's flagship pack are
//! Modrinth pins, and their bytes have never been on this disk. Downloading
//! them to read the manifest is half a gigabyte per pack per check, which is
//! why nothing checked.
//!
//! A zip's directory is at its tail, so two ranged requests reach any entry --
//! the tail, then the entry itself. That is what this does: a `Read + Seek`
//! over HTTP range requests, handed to the same zip reader every local jar goes
//! through, so nothing here parses a zip structure by hand.
//!
//! Blocking by construction (the zip reader is a synchronous consumer), so
//! every call belongs on a blocking task -- [`HttpRanges`] enforces that by
//! blocking on the shared async client, which panics on a runtime worker.

use super::archive::read_zip_entry;
use anyhow::{Context, Result};
use std::io::{self, Read, Seek, SeekFrom};

/// Bytes fetched per miss. Large enough that a jar's whole central directory
/// arrives in one request (a 100-mod-class jar's runs a few tens of kB), small
/// enough that reading a 2 kB manifest out of an 80 MB jar stays a rounding
/// error against downloading it.
const BLOCK: u64 = 256 * 1024;

/// Somewhere bytes can be read from by range. Implemented over HTTP in
/// production and over a `Vec<u8>` in tests, which is the whole reason it is a
/// trait: none of the zip handling below needs a network to be exercised.
pub trait RangeSource {
    /// Total size in bytes. Known up front -- a zip is read from its tail, so
    /// a source that cannot say how long it is cannot be read at all.
    fn len(&self) -> u64;
    /// `len` bytes from `from`. May return fewer only at the end of the file.
    fn fetch(&self, from: u64, len: u64) -> Result<Vec<u8>>;
}

/// The first of `names` the archive carries, decompressed, with the name it
/// was found under -- the caller reads a `mods.toml` and a `fabric.mod.json`
/// differently. `None` when it carries none of them: an ordinary answer (a jar
/// with no modern manifest), not an error.
pub fn read_entry(src: &dyn RangeSource, names: &[&str]) -> Result<Option<(String, Vec<u8>)>> {
    if src.len() == 0 {
        return Ok(None);
    }
    let mut zip = zip::ZipArchive::new(Windowed::new(src)).context("opening the remote jar")?;
    for name in names {
        let Ok(mut entry) = zip.by_name(name) else {
            continue;
        };
        let size = entry.size();
        return read_zip_entry(&mut entry, size, name).map(|b| Some((name.to_string(), b)));
    }
    Ok(None)
}

/// A `Read + Seek` view of a [`RangeSource`], holding the last block it
/// fetched. The zip reader seeks to the tail, scans back for the end-of-
/// directory record, walks the directory forward and then jumps to one entry:
/// a single cached block absorbs each of those runs, so the whole read costs
/// two or three requests.
struct Windowed<'a> {
    src: &'a dyn RangeSource,
    pos: u64,
    /// `(offset, bytes)` of what was last fetched.
    block: (u64, Vec<u8>),
}

impl<'a> Windowed<'a> {
    fn new(src: &'a dyn RangeSource) -> Self {
        Self {
            src,
            pos: 0,
            block: (0, Vec::new()),
        }
    }

    fn cached(&self, pos: u64) -> bool {
        let (start, bytes) = &self.block;
        pos >= *start && pos < *start + bytes.len() as u64
    }

    fn load(&mut self, pos: u64) -> io::Result<()> {
        let len = BLOCK.min(self.src.len().saturating_sub(pos));
        let bytes = self.src.fetch(pos, len).map_err(io::Error::other)?;
        if bytes.is_empty() {
            return Err(io::Error::other(format!(
                "range request at {pos} came back empty"
            )));
        }
        self.block = (pos, bytes);
        Ok(())
    }
}

impl Read for Windowed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.src.len() {
            return Ok(0);
        }
        if !self.cached(self.pos) {
            self.load(self.pos)?;
        }
        let (start, bytes) = &self.block;
        let offset = (self.pos - start) as usize;
        let n = buf.len().min(bytes.len() - offset);
        buf[..n].copy_from_slice(&bytes[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Windowed<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.src.len() as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the file",
            ));
        }
        self.pos = pos as u64;
        Ok(self.pos)
    }
}

/// A [`RangeSource`] over a URL, fetched with the mirror's own HTTP client.
///
/// Blocks the calling thread on the async client, so it must run on a blocking
/// task -- `Handle::block_on` panics on a runtime worker, which turns a misuse
/// into a test failure rather than a stalled server.
pub struct HttpRanges {
    modrinth: std::sync::Arc<super::modrinth::Modrinth>,
    handle: tokio::runtime::Handle,
    url: String,
    len: u64,
}

impl HttpRanges {
    /// Ask the server how long the file is and open a source over it. `None`
    /// when the server will not say, or serves the whole file regardless of the
    /// range asked for -- neither can be read this way, and pretending
    /// otherwise would download the jar a block at a time.
    pub async fn open(
        modrinth: std::sync::Arc<super::modrinth::Modrinth>,
        url: &str,
    ) -> Result<Option<Self>> {
        let Some(len) = modrinth.ranged_length(url).await? else {
            return Ok(None);
        };
        Ok(Some(Self {
            modrinth,
            handle: tokio::runtime::Handle::current(),
            url: url.to_string(),
            len,
        }))
    }
}

impl RangeSource for HttpRanges {
    fn len(&self) -> u64 {
        self.len
    }

    fn fetch(&self, from: u64, len: u64) -> Result<Vec<u8>> {
        let (modrinth, url) = (self.modrinth.clone(), self.url.clone());
        self.handle
            .block_on(async move { modrinth.fetch_range(&url, from, len).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// The same bytes, served by range, with a count of how many ranges it took.
    struct InMemory {
        bytes: Vec<u8>,
        fetches: Cell<usize>,
    }

    impl RangeSource for InMemory {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn fetch(&self, from: u64, len: u64) -> Result<Vec<u8>> {
            self.fetches.set(self.fetches.get() + 1);
            let from = from as usize;
            let to = (from + len as usize).min(self.bytes.len());
            Ok(self.bytes[from.min(self.bytes.len())..to].to_vec())
        }
    }

    fn source(bytes: Vec<u8>) -> InMemory {
        InMemory {
            bytes,
            fetches: Cell::new(0),
        }
    }

    #[test]
    fn the_named_entry_comes_back_out_of_a_remote_jar() {
        let jar = super::super::classfile::fixtures::jar(&[
            ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n"),
            (
                "META-INF/neoforge.mods.toml",
                br#"[[mods]]
modId="jei""#,
            ),
        ]);
        let src = source(jar);
        let (name, got) = read_entry(&src, &["META-INF/neoforge.mods.toml", "META-INF/mods.toml"])
            .unwrap()
            .expect("the jar carries it");
        assert_eq!(name, "META-INF/neoforge.mods.toml");
        assert!(String::from_utf8_lossy(&got).contains("modId=\"jei\""));
    }

    // The first name that is present wins, so a NeoForge jar carrying both
    // manifests answers with its own rather than the legacy one.
    #[test]
    fn names_are_tried_in_order() {
        let jar = super::super::classfile::fixtures::jar(&[
            ("META-INF/mods.toml", b"legacy"),
            ("META-INF/neoforge.mods.toml", b"modern"),
        ]);
        let src = source(jar);
        let (_, got) = read_entry(&src, &["META-INF/neoforge.mods.toml", "META-INF/mods.toml"])
            .unwrap()
            .unwrap();
        assert_eq!(got, b"modern");
    }

    // A jar with no modern manifest is an ordinary answer: 1.12-era jars carry
    // mcmod.info and nothing else, and that is not a failure to report.
    #[test]
    fn a_jar_without_the_entry_is_not_an_error() {
        let jar = super::super::classfile::fixtures::jar(&[("mcmod.info", b"[]")]);
        let src = source(jar);
        assert!(
            read_entry(&src, &["META-INF/neoforge.mods.toml"])
                .unwrap()
                .is_none()
        );
    }

    // The point of the exercise: a big jar costs a handful of ranges, not its
    // own size. The filler entry is incompressible so the archive really is
    // megabytes on the wire.
    #[test]
    fn a_large_jar_costs_a_few_ranges_rather_than_a_download() {
        // xorshift rather than a counter: a jar's payload is compressed, and a
        // pattern the deflater eats would leave a fixture that is only large
        // before it is written.
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let filler: Vec<u8> = (0..4_000_000u32)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        let jar = super::super::classfile::fixtures::jar(&[
            ("assets/big.bin", filler.as_slice()),
            ("META-INF/neoforge.mods.toml", b"modId=\"far\""),
        ]);
        let size = jar.len();
        let src = source(jar);
        let (_, got) = read_entry(&src, &["META-INF/neoforge.mods.toml"])
            .unwrap()
            .unwrap();
        assert_eq!(got, b"modId=\"far\"");
        assert!(size > 3_000_000, "the fixture is actually large: {size}");
        assert!(
            src.fetches.get() <= 4,
            "reading a 2 kB entry took {} ranges",
            src.fetches.get()
        );
    }

    #[test]
    fn bytes_that_are_not_a_zip_are_an_error_not_a_silent_none() {
        let src = source(b"not a zip at all".to_vec());
        assert!(read_entry(&src, &["META-INF/mods.toml"]).is_err());
        // and an empty file answers nothing rather than failing
        let empty = source(Vec::new());
        assert!(
            read_entry(&empty, &["META-INF/mods.toml"])
                .unwrap()
                .is_none()
        );
    }
}
