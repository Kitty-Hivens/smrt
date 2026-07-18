//! Prints the classifier evidence for each corpus jar. Dev instrument for
//! tuning the blanket client-surface rule; not part of the product.
fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: evidence_dump <dir-with-jars>");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jar"))
        .collect();
    files.sort();
    for p in files {
        let bytes = std::fs::read(&p).unwrap();
        let r = smrt::authoring::harvest::read_jar(&bytes);
        let e = &r.bytecode.evidence;
        println!(
            "{}\t{:?}\t{:?}\t{:?}\tcontent={} client={} mc={} distc={} dists={} proxy={} classes={}",
            p.file_name().unwrap().to_string_lossy(),
            r.bytecode.kind,
            r.bytecode.side,
            r.bytecode.match_policy,
            e.content_classes,
            e.client_classes,
            e.mc_touching,
            e.dist_client_classes,
            e.dist_server_classes,
            e.sided_proxy,
            e.classes,
        );
    }
}
