use std::io::Write;
use std::time::Instant;

use mine_utils_core::{scan_packs, IconCache};

#[test]
#[ignore = "release benchmark; creates 80,000 entries"]
fn scan_large_sources() {
    let root = tempfile::tempdir().unwrap();
    let icons = IconCache::new(&root.path().join("icons"));
    let meta = br#"{"pack":{"pack_format":61,"description":"benchmark"}}"#;
    let zips = root.path().join("zips");
    std::fs::create_dir(&zips).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(zips.join("Large.zip")).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("pack.mcmeta", options).unwrap();
    zip.write_all(meta).unwrap();
    for i in 1..60_000 {
        zip.start_file(format!("data/test/{i}.json"), options)
            .unwrap();
    }
    zip.finish().unwrap();
    let dirs = root.path().join("dirs");
    let folder = dirs.join("Large");
    std::fs::create_dir_all(folder.join("data/test")).unwrap();
    std::fs::write(folder.join("pack.mcmeta"), meta).unwrap();
    for i in 1..20_000 {
        std::fs::write(folder.join(format!("data/test/{i}.json")), b"").unwrap();
    }
    for (label, packs) in [("zip-60000", zips), ("folder-20000", dirs)] {
        let mut total = 0.0;
        for _ in 0..5 {
            let start = Instant::now();
            let listing = scan_packs(&packs, &icons).unwrap();
            total += start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(listing.packs[0].description, "benchmark");
            assert_eq!(listing.count, 1);
        }
        println!("{label}: {:.3} ms average of 5", total / 5.0);
    }
}
