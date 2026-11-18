//! Muestra de pruebas (T03): copia los primeros `n` discos de un dump a un XML pequeño.
//!
//! Corta a nivel de bytes, sin interpretar el XML: copia todo hasta el `n`-ésimo
//! `</release>` y cierra con `</releases>`. Así la muestra conserva el formato exacto del dump.

use std::io::{self, BufRead, Write};

const CLOSE: &[u8] = b"</release>";

/// Copia de `input` (XML ya descomprimido) a `out` los primeros `n` discos. Devuelve cuántos
/// se copiaron (menos de `n` si el dump es más corto).
pub fn extract_sample<R: BufRead, W: Write>(
    mut input: R,
    mut out: W,
    n: usize,
) -> io::Result<usize> {
    let mut count = 0;
    let mut chunk = Vec::with_capacity(4096);
    while count < n {
        chunk.clear();
        // Cada trozo acaba en '>', así que una etiqueta de cierre nunca queda partida.
        if input.read_until(b'>', &mut chunk)? == 0 {
            break;
        }
        out.write_all(&chunk)?;
        if chunk.ends_with(CLOSE) {
            count += 1;
        }
    }
    if count == n {
        out.write_all(b"\n</releases>\n")?;
    }
    out.flush()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discogs::ReleaseReader;

    #[test]
    fn corta_en_el_disco_n() {
        let xml = b"<?xml version=\"1.0\"?><releases><release id=\"1\"><title>a</title></release>\n<release id=\"2\"><title>b</title></release>\n<release id=\"3\"></release></releases>";
        let mut out = Vec::new();
        assert_eq!(extract_sample(&xml[..], &mut out, 2).unwrap(), 2);
        let ids: Vec<u64> = ReleaseReader::new(&out[..])
            .map(|r| r.unwrap().id)
            .collect();
        assert_eq!(ids, [1, 2]);
        let s = String::from_utf8(out).unwrap();
        assert!(s.ends_with("</releases>\n") && !s.contains("id=\"3\""));
    }

    #[test]
    fn dump_corto() {
        let xml = b"<releases><release id=\"1\"></release></releases>";
        let mut out = Vec::new();
        assert_eq!(extract_sample(&xml[..], &mut out, 5).unwrap(), 1);
        assert_eq!(out, xml);
    }
}
