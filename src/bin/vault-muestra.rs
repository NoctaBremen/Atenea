//! `vault-muestra <dump.xml.gz> <salida.xml> [n]`: extrae los primeros `n` discos (2000 por
//! defecto) de un dump de Discogs a un XML pequeño para los tests (T03).

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

use anyhow::{Context, bail};
use flate2::read::MultiGzDecoder;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || args.len() > 3 {
        bail!("uso: vault-muestra <dump.xml.gz> <salida.xml> [n]");
    }
    let input = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    let n: usize = match args.get(2) {
        Some(s) => s.parse().context("n debe ser un número")?,
        None => 2000,
    };
    let file =
        File::open(&input).with_context(|| format!("no se puede abrir {}", input.display()))?;
    let out = BufWriter::new(
        File::create(&output).with_context(|| format!("no se puede crear {}", output.display()))?,
    );
    let copied = if input.extension().is_some_and(|e| e == "gz") {
        vault::discogs::sample::extract_sample(BufReader::new(MultiGzDecoder::new(file)), out, n)
    } else {
        vault::discogs::sample::extract_sample(BufReader::new(file), out, n)
    }?;
    eprintln!("{copied} discos copiados a {}", output.display());
    Ok(())
}
