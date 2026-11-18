//! Catálogo género → estilos de Discogs, incluido en el programa (T06).
//!
//! Sirve para el filtro en árbol del diálogo de importación sin tener que recorrer el dump
//! antes. Se genera una vez con `atenea-catalogo` (que solo mira géneros y estilos) y se guarda
//! en `data/catalogo.tsv`: una línea `Género<TAB>Estilo` por par, `#` para comentarios.

use std::collections::{BTreeMap, BTreeSet};

/// El catálogo incluido en el binario.
pub const BUILTIN_TSV: &str = include_str!("../data/catalogo.tsv");

/// Géneros con sus estilos, ordenados alfabéticamente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    /// Género → estilos.
    pub genres: BTreeMap<String, BTreeSet<String>>,
}

impl Catalog {
    /// Lee un catálogo en formato TSV. Las líneas mal formadas se ignoran.
    pub fn parse(tsv: &str) -> Self {
        let mut c = Self::default();
        for line in tsv.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split('\t');
            let (Some(g), Some(s)) = (parts.next(), parts.next()) else {
                continue;
            };
            let (g, s) = (g.trim(), s.trim());
            if g.is_empty() {
                continue;
            }
            let entry = c.genres.entry(g.to_owned()).or_default();
            if !s.is_empty() {
                entry.insert(s.to_owned());
            }
        }
        c
    }

    /// El catálogo incluido.
    pub fn builtin() -> Self {
        Self::parse(BUILTIN_TSV)
    }

    /// Añade un par.
    pub fn add(&mut self, genre: &str, style: Option<&str>) {
        let e = self.genres.entry(genre.to_owned()).or_default();
        if let Some(s) = style {
            e.insert(s.to_owned());
        }
    }

    /// Escribe el catálogo en TSV, con cabecera.
    pub fn to_tsv(&self, header: &str) -> String {
        let mut s = String::new();
        for l in header.lines() {
            s.push_str("# ");
            s.push_str(l);
            s.push('\n');
        }
        for (g, styles) in &self.genres {
            if styles.is_empty() {
                s.push_str(&format!("{g}\t\n"));
            }
            for st in styles {
                s.push_str(&format!("{g}\t{st}\n"));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incluido_y_formato() {
        let c = Catalog::builtin();
        assert_eq!(c.genres.len(), 15);
        assert!(c.genres["Electronic"].contains("Tech House"));
        // Un mismo estilo puede colgar de dos géneros.
        assert!(c.genres["Rock"].contains("Experimental"));
        assert!(c.genres["Electronic"].contains("Experimental"));
        let again = Catalog::parse(&c.to_tsv("cabecera\nsegunda"));
        assert_eq!(again, c);
        // Sin tabulador o sin género: se ignora. Género sin estilo: género vacío.
        let p = Catalog::parse("# x\nA\tB\nmal\nA\tC\n\tZ\nSolo\t\n");
        assert_eq!(p.genres.len(), 2);
        assert_eq!(p.genres["A"].len(), 2);
        assert!(p.genres["Solo"].is_empty());
    }
}
