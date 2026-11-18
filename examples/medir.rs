//! Mide los tiempos de búsqueda sobre una base: `cargo run --release --example medir -- base.db`.

use std::time::Instant;

use atenea::db::{Base, Filtros, Sort, SortKey};

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("falta la base"))?;
    let b = Base::open(path.as_ref(), None)?;
    let casos: Vec<(&str, Filtros, Sort)> = vec![
        ("todo, por artista", Filtros::default(), Sort::default()),
        (
            "todo, por año desc",
            Filtros::default(),
            Sort {
                key: SortKey::Anyo,
                desc: true,
            },
        ),
        (
            "texto «night»",
            Filtros {
                texto: "night".into(),
                ..Default::default()
            },
            Sort::default(),
        ),
        (
            "estilo Tech House",
            Filtros {
                estilos: vec!["Tech House".into()],
                ..Default::default()
            },
            Sort::default(),
        ),
        (
            "12\" + 1990–2005 + vídeos",
            Filtros {
                formatos: vec!["12\"".into()],
                anyo_desde: Some(1990),
                anyo_hasta: Some(2005),
                solo_con_videos: true,
                ..Default::default()
            },
            Sort::default(),
        ),
        (
            "país + género",
            Filtros {
                pais: "Germany".into(),
                generos: vec!["Electronic".into()],
                ..Default::default()
            },
            Sort::default(),
        ),
    ];
    for (name, f, s) in casos {
        let t = Instant::now();
        let ids = b.ids(&f, s, None)?;
        let t_ids = t.elapsed();
        let t = Instant::now();
        let rows = b.rows(&ids[..ids.len().min(200)])?;
        println!(
            "{name:28} {:>8} ids en {:>6.0} ms, página de {} filas en {:.1} ms",
            ids.len(),
            t_ids.as_secs_f64() * 1000.0,
            rows.len(),
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
