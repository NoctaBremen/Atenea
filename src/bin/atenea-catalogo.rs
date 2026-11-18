//! `atenea-catalogo <dump.xml.gz>`: recorre un dump y escribe en la salida estándar el
//! catálogo género → estilo (formato de `data/catalogo.tsv`). Se ejecuta una vez para
//! regenerar el catálogo incluido en el programa.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::bail;
use atenea::catalog::Catalog;
use atenea::db::import::dump_date;
use atenea::discogs::open_dump;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dump] = args.as_slice() else {
        bail!("uso: atenea-catalogo <dump.xml.gz> > data/catalogo.tsv");
    };
    let path = PathBuf::from(dump);
    let mut cat = Catalog::default();
    // Discogs no dice de qué género cuelga cada estilo. Los discos con un solo género lo dejan
    // claro; los que tienen varios son ambiguos (Electronic + Funk / Soul con Deep House y
    // Disco) y solo se usan para estilos que nunca salen en un disco de un solo género.
    let mut ambiguous: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut n = 0u64;
    for rel in open_dump(&path)? {
        let rel = rel?;
        n += 1;
        if n.is_multiple_of(500_000) {
            eprintln!("{n} discos…");
        }
        for g in &rel.genres {
            cat.add(g, None);
        }
        if let [g] = rel.genres.as_slice() {
            for st in &rel.styles {
                cat.add(g, Some(st));
            }
        } else {
            for st in &rel.styles {
                ambiguous
                    .entry(st.clone())
                    .or_default()
                    .extend(rel.genres.iter().cloned());
            }
        }
    }
    let known: BTreeSet<String> = cat.genres.values().flatten().cloned().collect();
    for (st, genres) in ambiguous {
        if !known.contains(&st) {
            for g in genres {
                cat.add(&g, Some(&st));
            }
        }
    }
    let date = dump_date(&path).unwrap_or_else(|| "?".into());
    print!(
        "{}",
        cat.to_tsv(&format!(
            "Catálogo género → estilo de Discogs, generado con atenea-catalogo\n\
             desde el dump del {date} ({n} discos)."
        ))
    );
    Ok(())
}
