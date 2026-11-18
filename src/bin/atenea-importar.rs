//! `atenea-importar`: importa un dump de Discogs a una base de Atenea desde la terminal.
//!
//! ```text
//! atenea-importar <dump.xml.gz> <salida.db> [--nombre N] [--formato Vinyl]...
//!                 [--genero Electronic]... [--estilo "Electronic/Tech House"]...
//!                 [--desde 1990] [--hasta 2026]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use atenea::db::{ImportFilter, ImportOptions, Progress, import};

fn parse_args(args: &[String]) -> anyhow::Result<ImportOptions> {
    let mut pos = Vec::new();
    let mut filter = ImportFilter::default();
    let mut name = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().with_context(|| format!("falta el valor de {a}"));
        match a.as_str() {
            "--nombre" => name = Some(val()?.clone()),
            "--formato" => filter.formats.push(val()?.clone()),
            "--genero" => filter.genres.push(val()?.clone()),
            "--estilo" => {
                let v = val()?;
                let (g, s) = v
                    .split_once('/')
                    .with_context(|| format!("--estilo espera «Género/Estilo», no «{v}»"))?;
                filter.styles.push((g.trim().into(), s.trim().into()));
            }
            "--desde" => filter.year_from = Some(val()?.parse().context("--desde: año")?),
            "--hasta" => filter.year_to = Some(val()?.parse().context("--hasta: año")?),
            s if s.starts_with("--") => bail!("opción desconocida: {s}"),
            _ => pos.push(a.clone()),
        }
    }
    let [dump, output] = pos.as_slice() else {
        bail!(
            "uso: atenea-importar <dump.xml.gz> <salida.db> [--nombre N] [--formato F]... \
             [--genero G]... [--estilo G/E]... [--desde AAAA] [--hasta AAAA]"
        );
    };
    let output = PathBuf::from(output);
    let name = name.unwrap_or_else(|| {
        output
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    Ok(ImportOptions {
        dump: dump.into(),
        output,
        name,
        filter,
    })
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = parse_args(&args)?;
    eprintln!(
        "Importando {} → {} ({})",
        opts.dump.display(),
        opts.output.display(),
        opts.filter.describe()
    );
    let progress = Arc::new(Progress::default());
    let done = Arc::new(AtomicBool::new(false));
    let reporter = {
        let (p, done) = (Arc::clone(&progress), Arc::clone(&done));
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                let secs = start.elapsed().as_secs_f64().max(0.001);
                let seen = p.seen.load(Ordering::Relaxed);
                eprint!(
                    "\r{:5.1} %  {} leídos  {} importados  {:.0} discos/s   ",
                    p.fraction() * 100.0,
                    seen,
                    p.imported.load(Ordering::Relaxed),
                    seen as f64 / secs
                );
            }
            eprintln!();
        })
    };
    let result = import(&opts, &progress, &AtomicBool::new(false));
    done.store(true, Ordering::Relaxed);
    let _ = reporter.join();
    let stats = result?;
    eprintln!(
        "Hecho: {} de {} discos en {} s",
        stats.imported, stats.seen, stats.seconds
    );
    Ok(())
}
