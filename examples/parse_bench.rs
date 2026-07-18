//! Times the harvest per-jar reader over a corpus directory. Dev instrument
//! for the stage-I before/after comparison; not part of the product.
use std::time::Instant;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: parse_bench <dir-with-jars>");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jar"))
        .collect();
    files.sort();
    let blobs: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let t = Instant::now();
    let mut acc = 0usize;
    for b in &blobs {
        let r = smrt::authoring::harvest::read_jar(b);
        acc += r.bytecode.owned.len();
    }
    let dt = t.elapsed();
    println!(
        "{} jars, {:?} total, {:.1} ms/jar, checksum {}",
        blobs.len(),
        dt,
        dt.as_millis() as f64 / blobs.len() as f64,
        acc
    );
}
