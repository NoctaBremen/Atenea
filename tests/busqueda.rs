#![allow(clippy::unwrap_used, clippy::expect_used)]
//! T09, T12, T13, T18–T22: consultas sobre la muestra importada.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use atenea::db::{
    Base, Filtros, ImportFilter, ImportOptions, Mark, Progress, Sort, SortKey, import,
};
use atenea::discogs::{Release, open_dump};

const SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/sample_releases.xml"
);

/// Importa la muestra una sola vez para todos los tests de este archivo.
fn base_path() -> &'static Path {
    static P: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    &P.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("muestra.db");
        let opts = ImportOptions {
            dump: SAMPLE.into(),
            output: out.clone(),
            name: "Muestra".into(),
            filter: ImportFilter::default(),
        };
        import(
            &opts,
            &Arc::new(Progress::default()),
            &AtomicBool::new(false),
        )
        .unwrap();
        (dir, out)
    })
    .1
}

fn releases() -> &'static Vec<Release> {
    static R: OnceLock<Vec<Release>> = OnceLock::new();
    R.get_or_init(|| {
        open_dump(Path::new(SAMPLE))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    })
}

fn base() -> Base {
    Base::open(base_path(), None).unwrap()
}

fn ids(b: &Base, f: Filtros) -> Vec<i64> {
    let mut v = b.ids(&f, Sort::default(), None).unwrap();
    v.sort_unstable();
    v
}

fn expected(pred: impl Fn(&Release) -> bool) -> Vec<i64> {
    let mut v: Vec<i64> = releases()
        .iter()
        .filter(|r| pred(r))
        .map(|r| r.id as i64)
        .collect();
    v.sort_unstable();
    v
}

#[test]
fn info_de_la_base() {
    let b = base();
    assert_eq!(b.info().name, "Muestra");
    assert_eq!(b.info().release_count, 2000);
    assert_eq!(b.info().filter_text, "Todo");
}

#[test]
fn texto_con_frase_exacta_y_prefijo() {
    let b = base();
    assert_eq!(
        ids(
            &b,
            Filtros {
                texto: "\"kalo tren\" axis".into(),
                ..Default::default()
            }
        ),
        vec![1, 4]
    );
    // Prefijo y sin distinguir acentos ni mayúsculas.
    assert_eq!(
        ids(
            &b,
            Filtros {
                texto: "dia noch".into(),
                ..Default::default()
            }
        ),
        vec![2]
    );
    // Por catno y por título de pista.
    assert_eq!(
        ids(
            &b,
            Filtros {
                texto: "\"OND 044\"".into(),
                ..Default::default()
            }
        ),
        vec![6]
    );
    assert!(
        ids(
            &b,
            Filtros {
                texto: "\"low orbit\"".into(),
                ..Default::default()
            }
        )
        .contains(&1)
    );
    // Puntuación suelta no rompe la consulta.
    assert_eq!(
        ids(
            &b,
            Filtros {
                texto: "kalo & - ( tren".into(),
                ..Default::default()
            }
        )
        .len(),
        3
    );
}

#[test]
fn cada_filtro_cuadra_con_el_xml() {
    let b = base();
    let has = |v: &[String], x: &str| v.iter().any(|s| s == x);

    assert_eq!(
        ids(
            &b,
            Filtros {
                anyo_desde: Some(1990),
                anyo_hasta: Some(1999),
                ..Default::default()
            }
        ),
        expected(|r| r.year().is_some_and(|y| (1990..=1999).contains(&y)))
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                pais: "Germany".into(),
                ..Default::default()
            }
        ),
        expected(|r| r.country == "Germany")
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                generos: vec!["Jazz".into(), "Reggae".into()],
                ..Default::default()
            }
        ),
        expected(|r| has(&r.genres, "Jazz") || has(&r.genres, "Reggae"))
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                estilos: vec!["Tech House".into()],
                ..Default::default()
            }
        ),
        expected(|r| has(&r.styles, "Tech House"))
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                formatos: vec!["7\"".into()],
                ..Default::default()
            }
        ),
        expected(|r| r
            .formats
            .iter()
            .any(|f| f.descriptions.iter().any(|d| d == "7\"")))
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                solo_con_videos: true,
                ..Default::default()
            }
        ),
        expected(|r| !r.videos.is_empty())
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                artista: "\"kalo tren\"".into(),
                ..Default::default()
            }
        ),
        vec![1, 4, 7]
    );
    assert_eq!(
        ids(
            &b,
            Filtros {
                sello: "ondas".into(),
                ..Default::default()
            }
        ),
        expected(|r| r.labels.iter().any(|l| l.name == "Ondas Records"))
    );
}

#[test]
fn combinaciones() {
    let b = base();
    let f = Filtros {
        estilos: vec!["Tech House".into()],
        formatos: vec!["12\"".into()],
        anyo_desde: Some(1995),
        anyo_hasta: Some(2005),
        solo_con_videos: true,
        ..Default::default()
    };
    let got = ids(&b, f);
    let want = expected(|r| {
        r.styles.iter().any(|s| s == "Tech House")
            && r.formats
                .iter()
                .any(|fm| fm.descriptions.iter().any(|d| d == "12\""))
            && r.year().is_some_and(|y| (1995..=2005).contains(&y))
            && !r.videos.is_empty()
    });
    assert!(got.contains(&1));
    assert_eq!(got, want);
    let none = ids(
        &b,
        Filtros {
            texto: "axis".into(),
            generos: vec!["Rock".into()],
            ..Default::default()
        },
    );
    assert!(none.is_empty());
}

#[test]
fn orden_y_filas() {
    let b = base();
    let f = Filtros {
        artista: "\"kalo tren\"".into(),
        ..Default::default()
    };
    let by_year = b
        .ids(
            &f,
            Sort {
                key: SortKey::Anyo,
                desc: false,
            },
            None,
        )
        .unwrap();
    assert_eq!(by_year, vec![1, 4, 7]);
    let desc = b
        .ids(
            &f,
            Sort {
                key: SortKey::Anyo,
                desc: true,
            },
            Some(2),
        )
        .unwrap();
    assert_eq!(desc, vec![7, 4]);
    // Sin año va al final en los dos sentidos.
    let amb = Filtros {
        estilos: vec!["Ambient".into()],
        ..Default::default()
    };
    let asc = b
        .ids(
            &amb,
            Sort {
                key: SortKey::Anyo,
                desc: false,
            },
            None,
        )
        .unwrap();
    let dsc = b
        .ids(
            &amb,
            Sort {
                key: SortKey::Anyo,
                desc: true,
            },
            None,
        )
        .unwrap();
    assert_eq!(asc.last(), Some(&8));
    assert_eq!(dsc.last(), Some(&8));

    let rows = b.rows(&[7, 999_999, 1]).unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![7, 1]);
    assert_eq!(rows[1].artist, "Kalo Tren");
    assert_eq!(rows[1].format, "Vinyl, 12\", 33 ⅓ RPM, EP");
    assert_eq!(rows[1].n_videos, 2);
    let all = b.ids(&Filtros::default(), Sort::default(), None).unwrap();
    assert_eq!(b.rows(&all).unwrap().len(), 2000);
}

#[test]
fn detalle() {
    let b = base();
    let d = b.detail(1).unwrap().unwrap();
    assert_eq!(d.labels, vec![("Ondas Records".into(), "OND 001".into())]);
    assert_eq!(d.tracks.len(), 3);
    assert_eq!(d.tracks[2].duration, "7:03");
    assert_eq!(d.credits, vec![("Producer".into(), "Mira Solvek".into())]);
    assert_eq!(d.videos.len(), 2);
    assert_eq!(d.videos[0].duration, 372);
    assert_eq!(d.master_id, Some(31));
    assert_eq!(d.discogs_url(), "https://www.discogs.com/release/1");
    assert!(b.detail(999_999).unwrap().is_none());

    // Discos parecidos, más del sello, más del artista.
    let p = d.filtros_parecidos().unwrap();
    assert_eq!(p.estilos, vec!["Tech House".to_owned()]);
    assert_eq!((p.anyo_desde, p.anyo_hasta), (Some(1996), Some(2000)));
    let ids = b.ids(&p, Sort::default(), None).unwrap();
    assert!(ids.contains(&1) && !ids.contains(&4), "el 4 es de 2001");
    let s = b
        .ids(&d.filtros_sello().unwrap(), Sort::default(), None)
        .unwrap();
    assert!(s.contains(&1) && s.contains(&6));
    let d2 = b.detail(2).unwrap().unwrap();
    assert_eq!(
        d2.artists,
        vec!["Dovra Mon".to_owned(), "Velquin".to_owned()]
    );
    let a = b
        .ids(&d.filtros_artista().unwrap(), Sort::default(), None)
        .unwrap();
    let mut a = a;
    a.sort_unstable();
    assert_eq!(a, vec![1, 4, 7]);
    assert!(
        b.genres()
            .unwrap()
            .iter()
            .any(|(g, n)| g == "Electronic" && *n > 100)
    );
    assert!(b.styles().unwrap().iter().any(|(s, _)| s == "Hard Bop"));
    assert!(b.countries().unwrap()[0].1 >= b.countries().unwrap()[1].1);
}

#[test]
fn datos_de_usuario_en_los_filtros() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.db");
    let b = Base::open(base_path(), Some(&user)).unwrap();
    let u = b.user();
    u.set_mark(1, Some(Mark::Interesa)).unwrap();
    u.set_mark(2, Some(Mark::NoInteresa)).unwrap();
    u.set_mark(3, Some(Mark::Volver)).unwrap();
    u.set_note(5, "Ojo al solo de contrabajo").unwrap();
    let m = u.create_suitcase("Techno House", "curva").unwrap();
    u.add_to_suitcase(m, 6).unwrap();
    u.add_to_suitcase(m, 9).unwrap();

    let with = |f: Filtros| ids(&b, f);
    assert_eq!(
        with(Filtros {
            marcadores: vec![Mark::Interesa, Mark::Volver],
            ..Default::default()
        }),
        vec![1, 3]
    );
    let hidden = with(Filtros {
        ocultar_no_interesa: true,
        ..Default::default()
    });
    assert_eq!(hidden.len(), 1999);
    assert!(!hidden.contains(&2));
    assert_eq!(
        with(Filtros {
            maleta: Some(m),
            ..Default::default()
        }),
        vec![6, 9]
    );
    assert_eq!(
        with(Filtros {
            texto: "contrabajo".into(),
            ..Default::default()
        }),
        vec![5]
    );
    assert_eq!(u.note(5).unwrap(), "Ojo al solo de contrabajo");

    // La base de Discogs es de solo lectura; la de usuario, no.
    drop(b);
    let b = Base::open(base_path(), Some(&user)).unwrap();
    assert_eq!(b.user().mark(1).unwrap(), Some(Mark::Interesa));
    let c = rusqlite::Connection::open_with_flags(
        format!("file:{}?mode=ro", base_path().display()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    assert!(c.execute("DELETE FROM releases WHERE id = 1", []).is_err());
}

#[test]
fn no_abre_lo_que_no_es_una_base() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.db");
    rusqlite::Connection::open(&p)
        .unwrap()
        .execute_batch("CREATE TABLE t(x)")
        .unwrap();
    assert!(Base::open(&p, None).is_err());
    assert!(Base::open(&dir.path().join("no.db"), None).is_err());
    std::fs::write(dir.path().join("txt.db"), "hola").unwrap();
    assert!(Base::open(&dir.path().join("txt.db"), None).is_err());
}
