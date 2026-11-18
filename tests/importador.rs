#![allow(clippy::unwrap_used, clippy::expect_used)]
//! T06: importar la muestra con y sin filtros; los recuentos cuadran con el XML.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rusqlite::Connection;
use vault::db::{ImportError, ImportFilter, ImportOptions, Progress, import};
use vault::discogs::{Release, open_dump};

const SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/sample_releases.xml"
);

fn releases() -> Vec<Release> {
    open_dump(Path::new(SAMPLE))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn run(dir: &Path, name: &str, filter: ImportFilter) -> Connection {
    let out = dir.join(format!("{name}.db"));
    let opts = ImportOptions {
        dump: SAMPLE.into(),
        output: out.clone(),
        name: name.into(),
        filter,
    };
    let progress = Arc::new(Progress::default());
    let stats = import(&opts, &progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(stats.seen, 2000);
    assert!((progress.fraction() - 1.0).abs() < 1e-9);
    Connection::open(out).unwrap()
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn sin_filtros_entra_todo() {
    let dir = tempfile::tempdir().unwrap();
    let c = run(dir.path(), "todo", ImportFilter::default());
    let rels = releases();
    assert_eq!(count(&c, "SELECT count(*) FROM releases"), 2000);
    let tracks: usize = rels.iter().map(|r| r.tracks.len()).sum();
    let videos: usize = rels.iter().map(|r| r.videos.len()).sum();
    assert_eq!(count(&c, "SELECT count(*) FROM tracks"), tracks as i64);
    assert_eq!(count(&c, "SELECT count(*) FROM videos"), videos as i64);
    assert_eq!(
        count(&c, "SELECT count(*) FROM releases WHERE n_videos > 0"),
        rels.iter().filter(|r| !r.videos.is_empty()).count() as i64
    );
    let meta: String = c
        .query_row(
            "SELECT value FROM meta WHERE key='release_count'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(meta, "2000");
    let (artist, label, catno, year): (String, String, String, i64) = c
        .query_row(
            "SELECT artist_line, label_line, catno_line, year FROM releases WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (artist.as_str(), label.as_str(), catno.as_str(), year),
        ("Kalo Tren", "Ondas Records", "OND 001", 1998)
    );
    assert_eq!(
        count(
            &c,
            "SELECT rowid FROM releases_fts WHERE releases_fts MATCH '\"kalo tren\" axis' AND rowid = 1"
        ),
        1
    );
}

#[test]
fn con_filtros_los_recuentos_cuadran() {
    let dir = tempfile::tempdir().unwrap();
    let rels = releases();

    let c = run(dir.path(), "vinilo", ImportFilter::all_vinyl());
    let vinyl = rels.iter().filter(|r| r.has_format("Vinyl")).count() as i64;
    assert_eq!(count(&c, "SELECT count(*) FROM releases"), vinyl);
    assert_eq!(
        count(&c, "SELECT count(*) FROM releases WHERE id IN (4, 7)"),
        0
    );

    let f = ImportFilter {
        formats: vec!["Vinyl".into()],
        styles: vec![("Electronic".into(), "Tech House".into())],
        genres: vec!["Rock".into()],
        year_from: Some(1990),
        year_to: Some(2026),
    };
    let c = run(dir.path(), "mezcla", f);
    let expected = rels
        .iter()
        .filter(|r| {
            let y = r.year().unwrap_or(0);
            r.has_format("Vinyl")
                && (1990..=2026).contains(&y)
                && (r.genres.iter().any(|g| g == "Rock")
                    || (r.genres.iter().any(|g| g == "Electronic")
                        && r.styles.iter().any(|s| s == "Tech House")))
        })
        .count() as i64;
    assert!(expected > 10, "{expected}");
    assert_eq!(count(&c, "SELECT count(*) FROM releases"), expected);
    assert_eq!(count(&c, "SELECT count(*) FROM releases WHERE id = 1"), 1);
    assert_eq!(
        count(&c, "SELECT count(*) FROM releases WHERE id = 3"),
        0,
        "1981"
    );
}

#[test]
fn cancelar_no_deja_base() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.db");
    let opts = ImportOptions {
        dump: SAMPLE.into(),
        output: out.clone(),
        name: "x".into(),
        filter: ImportFilter::default(),
    };
    let r = import(
        &opts,
        &Arc::new(Progress::default()),
        &AtomicBool::new(true),
    );
    assert!(matches!(r, Err(ImportError::Cancelled)));
    assert!(!out.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
