//! Dev probe: dump the modern-metadata readout of one jar.
//! `cargo run --example meta_probe -- <jar path>`
fn main() {
    let bytes = std::fs::read(std::env::args().nth(1).expect("jar path")).expect("read jar");
    let r = smrt::authoring::harvest::read_jar(&bytes);
    println!("modid        = {:?}", r.modmeta.modid);
    println!("display_name = {:?}", r.modmeta.display_name);
    println!("version      = {:?}", r.modmeta.version);
    println!("logo_file    = {:?}", r.modmeta.logo_file);
    println!("loader       = {:?}", r.facts.loader);
}
