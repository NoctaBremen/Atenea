#![allow(clippy::unwrap_used, clippy::expect_used)]
//! T04: el parser lee la muestra entera, también comprimida, con los discos fijos intactos.

use std::io::Write;
use std::path::Path;

use vault::discogs::{Release, open_dump};

const SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/sample_releases.xml"
);

fn read_all(path: &Path) -> Vec<Release> {
    open_dump(path).unwrap().collect::<Result<_, _>>().unwrap()
}

#[test]
fn lee_la_muestra() {
    let rels = read_all(Path::new(SAMPLE));
    assert_eq!(rels.len(), 2000);
    assert_eq!(rels[0].title, "Axis Point");
    assert_eq!(rels[1].title, "Día & Noche");
    assert_eq!(rels[1].artist_line(), "Dovra Mon & Velquin");
    assert_eq!(rels[4].notes, "Grabado en una sola sesión <directo>.");
    assert_eq!(rels[7].year(), None);
    assert!(rels.iter().all(|r| r.id > 0 && !r.title.is_empty()));
    let vinyl = rels.iter().filter(|r| r.has_format("Vinyl")).count();
    assert!(vinyl > 1400 && vinyl < 1800, "{vinyl}");
}

#[test]
fn lee_la_muestra_comprimida_igual() {
    let dir = tempfile::tempdir().unwrap();
    let gz = dir.path().join("discogs_20260101_releases.xml.gz");
    let mut enc = flate2::write::GzEncoder::new(
        std::fs::File::create(&gz).unwrap(),
        flate2::Compression::fast(),
    );
    enc.write_all(&std::fs::read(SAMPLE).unwrap()).unwrap();
    enc.finish().unwrap();
    assert_eq!(read_all(&gz), read_all(Path::new(SAMPLE)));
}
