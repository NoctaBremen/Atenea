//! Exportar una maleta a CSV (T21): una fila por disco con `release_id` y enlace de Discogs,
//! para comprar o importar a tu wantlist. Separador coma, comillas RFC 4180 y BOM UTF-8 para
//! que LibreOffice y Excel reconozcan los acentos.

use std::io::{self, Write};

use crate::db::base::{Row, discogs_url};

/// Cabecera del CSV.
pub const HEADER: [&str; 10] = [
    "release_id",
    "artista",
    "titulo",
    "sello",
    "catno",
    "anyo",
    "pais",
    "formato",
    "estilos",
    "enlace",
];

/// Escapa un campo CSV (comillas si lleva coma, comillas o saltos de línea).
pub fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// Escribe las filas como CSV.
pub fn write_csv<W: Write>(mut w: W, rows: &[Row]) -> io::Result<()> {
    w.write_all("\u{feff}".as_bytes())?;
    writeln!(w, "{}", HEADER.join(","))?;
    for r in rows {
        let year = r.year.map(|y| y.to_string()).unwrap_or_default();
        let cells = [
            r.id.to_string(),
            field(&r.artist),
            field(&r.title),
            field(&r.label),
            field(&r.catno),
            year,
            field(&r.country),
            field(&r.format),
            field(&r.styles),
            discogs_url(r.id),
        ];
        writeln!(w, "{}", cells.join(","))?;
    }
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_con_comillas_y_bom() {
        let rows = vec![Row {
            id: 2,
            artist: "Dovra Mon & Velquin".into(),
            title: "Día, \"Noche\"".into(),
            label: "Pax Tracks".into(),
            catno: "PAX-12".into(),
            year: Some(2003),
            format: "Vinyl, 12\", 45 RPM".into(),
            ..Default::default()
        }];
        let mut out = Vec::new();
        write_csv(&mut out, &rows).unwrap();
        let s = String::from_utf8(out).unwrap();
        let mut lines = s.lines();
        assert_eq!(
            lines.next().unwrap(),
            "\u{feff}release_id,artista,titulo,sello,catno,anyo,pais,formato,estilos,enlace"
        );
        assert_eq!(
            lines.next().unwrap(),
            "2,Dovra Mon & Velquin,\"Día, \"\"Noche\"\"\",Pax Tracks,PAX-12,2003,,\"Vinyl, 12\"\", 45 RPM\",,https://www.discogs.com/release/2"
        );
        assert!(lines.next().is_none());
    }
}
