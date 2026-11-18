//! Importador (T06): lee el dump en streaming y lo vuelca a SQLite por lotes, con filtros,
//! progreso y cancelación.
//!
//! Se escribe en un archivo temporal junto al destino y solo se renombra al terminar bien: una
//! importación cancelada o fallida no deja una base a medias.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use flate2::read::MultiGzDecoder;
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::schema;
use crate::discogs::{ParseError, Release, ReleaseReader};

/// Discos por transacción.
const BATCH: usize = 5_000;

/// Qué discos entran en la base. Todo es opcional: el filtro vacío lo importa todo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportFilter {
    /// Formatos admitidos («Vinyl», «CD»…). Vacío = todos.
    pub formats: Vec<String>,
    /// Géneros completos (todos sus estilos).
    pub genres: Vec<String>,
    /// Estilos concretos como (género, estilo): Electronic → Tech House.
    pub styles: Vec<(String, String)>,
    /// Año mínimo (incluido).
    pub year_from: Option<u16>,
    /// Año máximo (incluido).
    pub year_to: Option<u16>,
}

impl ImportFilter {
    /// Preajuste «Todo en vinilo»: todos los géneros, solo vinilo.
    pub fn all_vinyl() -> Self {
        Self {
            formats: vec!["Vinyl".into()],
            ..Self::default()
        }
    }

    /// ¿Entra este disco?
    ///
    /// - Formato: alguno de sus formatos está en la lista.
    /// - Género/estilo: si no se eligió ninguno, entran todos; si no, entra si tiene uno de los
    ///   géneros completos, o un par (género, estilo) elegido.
    /// - Años: con rango, los discos sin año quedan fuera.
    pub fn accepts(&self, r: &Release) -> bool {
        if !self.formats.is_empty()
            && !self
                .formats
                .iter()
                .any(|f| r.formats.iter().any(|rf| rf.name.eq_ignore_ascii_case(f)))
        {
            return false;
        }
        if !self.genres.is_empty() || !self.styles.is_empty() {
            let by_genre = r.genres.iter().any(|g| self.genres.contains(g));
            let by_style = self
                .styles
                .iter()
                .any(|(g, s)| r.genres.contains(g) && r.styles.contains(s));
            if !by_genre && !by_style {
                return false;
            }
        }
        if self.year_from.is_some() || self.year_to.is_some() {
            let Some(y) = r.year() else {
                return false;
            };
            if self.year_from.is_some_and(|f| y < f) || self.year_to.is_some_and(|t| y > t) {
                return false;
            }
        }
        true
    }

    /// Descripción corta para mostrar («Vinyl · Electronic → Tech House · 1990–2026»).
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.formats.is_empty() {
            parts.push(self.formats.join(", "));
        }
        let mut gs: Vec<String> = self.genres.clone();
        gs.extend(self.styles.iter().map(|(g, s)| format!("{g} → {s}")));
        if !gs.is_empty() {
            parts.push(gs.join(", "));
        }
        match (self.year_from, self.year_to) {
            (Some(f), Some(t)) => parts.push(format!("{f}–{t}")),
            (Some(f), None) => parts.push(format!("desde {f}")),
            (None, Some(t)) => parts.push(format!("hasta {t}")),
            (None, None) => {}
        }
        if parts.is_empty() {
            "Todo".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// Progreso que el importador va publicando (lo lee la interfaz desde otro hilo).
#[derive(Debug, Default)]
pub struct Progress {
    /// Bytes del archivo de entrada leídos (comprimidos si es `.gz`).
    pub bytes_read: AtomicU64,
    /// Tamaño del archivo de entrada.
    pub bytes_total: AtomicU64,
    /// Discos leídos del dump.
    pub seen: AtomicU64,
    /// Discos importados (los que pasan el filtro).
    pub imported: AtomicU64,
}

impl Progress {
    /// Fracción leída en [0, 1].
    pub fn fraction(&self) -> f64 {
        let total = self.bytes_total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.bytes_read.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0)
    }
}

/// Resultado de una importación terminada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportStats {
    /// Discos leídos.
    pub seen: u64,
    /// Discos importados.
    pub imported: u64,
    /// Segundos empleados.
    pub seconds: u64,
}

/// Error de importación.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// El usuario canceló.
    #[error("importación cancelada")]
    Cancelled,
    /// Error leyendo el dump.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// Error de E/S.
    #[error("error de archivo: {0}")]
    Io(#[from] io::Error),
    /// Error de SQLite.
    #[error("error de base de datos: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// Opciones de una importación.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Dump de entrada (`.xml.gz` o `.xml`).
    pub dump: PathBuf,
    /// Base de salida (se sustituye si existe).
    pub output: PathBuf,
    /// Nombre de la base («Todo en vinilo»).
    pub name: String,
    /// Filtro.
    pub filter: ImportFilter,
}

/// Lector que cuenta los bytes que pasan por él.
struct Counting<R> {
    inner: R,
    count: Arc<Progress>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.bytes_read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Fecha del dump a partir del nombre (`discogs_20261101_releases.xml.gz` → `2026-11-01`).
pub fn dump_date(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let digits: String = name
        .split('_')
        .find(|p| p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit()))?
        .to_owned();
    Some(format!(
        "{}-{}-{}",
        &digits[..4],
        &digits[4..6],
        &digits[6..]
    ))
}

/// Importa un dump. Bloquea: hay que llamarlo desde un hilo aparte. `cancel` se consulta
/// entre discos.
pub fn import(
    opts: &ImportOptions,
    progress: &Arc<Progress>,
    cancel: &AtomicBool,
) -> Result<ImportStats, ImportError> {
    let start = Instant::now();
    let file = File::open(&opts.dump)?;
    progress
        .bytes_total
        .store(file.metadata()?.len(), Ordering::Relaxed);
    let counted = Counting {
        inner: file,
        count: Arc::clone(progress),
    };
    let gz = opts
        .dump
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gz"));
    let input: Box<dyn BufRead> = if gz {
        Box::new(BufReader::with_capacity(
            1 << 20,
            MultiGzDecoder::new(BufReader::with_capacity(1 << 20, counted)),
        ))
    } else {
        Box::new(BufReader::with_capacity(1 << 20, counted))
    };

    if let Some(dir) = opts.output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = crate::fsutil::temp_path_for(&opts.output);
    let result = import_into(&tmp, input, opts, progress, cancel, start);
    match result {
        Ok(stats) => {
            std::fs::rename(&tmp, &opts.output)?;
            Ok(stats)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn import_into(
    db_path: &Path,
    input: Box<dyn BufRead>,
    opts: &ImportOptions,
    progress: &Progress,
    cancel: &AtomicBool,
    start: Instant,
) -> Result<ImportStats, ImportError> {
    let mut conn = Connection::open(db_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -262144;
         PRAGMA page_size = 8192;",
    )?;
    schema::create_tables(&conn)?;

    let mut writer = Writer::default();
    let mut tx = conn.transaction()?;
    let mut in_batch = 0;
    for rel in ReleaseReader::new(input) {
        if cancel.load(Ordering::Relaxed) {
            return Err(ImportError::Cancelled);
        }
        let rel = rel?;
        progress.seen.fetch_add(1, Ordering::Relaxed);
        if !opts.filter.accepts(&rel) {
            continue;
        }
        writer.insert(&tx, &rel)?;
        progress.imported.fetch_add(1, Ordering::Relaxed);
        in_batch += 1;
        if in_batch == BATCH {
            tx.commit()?;
            tx = conn.transaction()?;
            in_batch = 0;
        }
    }
    tx.commit()?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ImportError::Cancelled);
    }

    schema::create_indexes(&conn)?;
    let seen = progress.seen.load(Ordering::Relaxed);
    let imported = progress.imported.load(Ordering::Relaxed);
    let filter_json = serde_json::to_string(&opts.filter).unwrap_or_default();
    let meta = [
        ("name", opts.name.clone()),
        ("dump_file", opts.dump.display().to_string()),
        ("dump_date", dump_date(&opts.dump).unwrap_or_default()),
        ("filter", filter_json),
        ("filter_text", opts.filter.describe()),
        ("release_count", imported.to_string()),
        ("seen_count", seen.to_string()),
        ("imported_at", now_iso()),
    ];
    for (k, v) in meta {
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            params![k, v],
        )?;
    }
    conn.execute_batch("PRAGMA journal_mode = DELETE;")?;
    drop(conn);
    Ok(ImportStats {
        seen,
        imported,
        seconds: start.elapsed().as_secs(),
    })
}

/// Fecha y hora UTC actuales en ISO 8601 (sin dependencias externas).
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Algoritmo de días civiles (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Inserta discos con sentencias preparadas y mantiene los ids de géneros y estilos.
#[derive(Default)]
struct Writer {
    genres: HashMap<String, i64>,
    styles: HashMap<String, i64>,
}

impl Writer {
    fn id_for(
        tx: &Transaction<'_>,
        map: &mut HashMap<String, i64>,
        table: &str,
        name: &str,
    ) -> rusqlite::Result<i64> {
        if let Some(&id) = map.get(name) {
            return Ok(id);
        }
        let id = map.len() as i64 + 1;
        tx.prepare_cached(&format!("INSERT INTO {table}(id, name) VALUES (?1, ?2)"))?
            .execute(params![id, name])?;
        map.insert(name.to_owned(), id);
        Ok(id)
    }

    fn insert(&mut self, tx: &Transaction<'_>, r: &Release) -> rusqlite::Result<()> {
        let id = r.id as i64;
        let label_line = dedup_join(r.labels.iter().map(|l| l.name.as_str()), ", ");
        let catno_line = dedup_join(r.labels.iter().map(|l| l.catno.as_str()), ", ");
        let format_line = r
            .formats
            .iter()
            .map(|f| {
                let mut s = String::new();
                if f.qty > 1 {
                    s.push_str(&format!("{}×", f.qty));
                }
                s.push_str(&f.name);
                if !f.descriptions.is_empty() {
                    s.push_str(", ");
                    s.push_str(&f.descriptions.join(", "));
                }
                s
            })
            .collect::<Vec<_>>()
            .join(" + ");
        let artist_line = r.artist_line();
        tx.prepare_cached(
            "INSERT OR REPLACE INTO releases(id, title, artist_line, label_line, catno_line,
                format_line, genre_line, style_line, country, released, year, notes, master_id,
                n_videos, fmt_flags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?
        .execute(params![
            id,
            r.title,
            artist_line,
            label_line,
            catno_line,
            format_line,
            r.genres.join(", "),
            r.styles.join(", "),
            r.country,
            r.released,
            r.year(),
            r.notes,
            r.master_id.map(|m| m as i64),
            r.videos.len() as i64,
            schema::format_flags(
                r.formats
                    .iter()
                    .flat_map(|f| f.descriptions.iter().map(String::as_str))
            ),
        ])?;

        for (i, a) in r.artists.iter().enumerate() {
            tx.prepare_cached("INSERT OR IGNORE INTO artists(id, name) VALUES (?1, ?2)")?
                .execute(params![a.id as i64, a.name])?;
            tx.prepare_cached(
                "INSERT OR REPLACE INTO release_artists(release_id, position, artist_id, anv, join_text)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![id, i as i64, a.id as i64, a.anv, a.join])?;
        }
        for (i, l) in r.labels.iter().enumerate() {
            tx.prepare_cached("INSERT OR IGNORE INTO labels(id, name) VALUES (?1, ?2)")?
                .execute(params![l.id as i64, l.name])?;
            tx.prepare_cached(
                "INSERT OR REPLACE INTO release_labels(release_id, position, label_id, catno)
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![id, i as i64, l.id as i64, l.catno])?;
        }
        for (i, f) in r.formats.iter().enumerate() {
            tx.prepare_cached(
                "INSERT OR REPLACE INTO formats(release_id, position, name, qty, text, descriptions)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                id,
                i as i64,
                f.name,
                f.qty,
                f.text,
                f.descriptions.join("|")
            ])?;
        }
        for g in &r.genres {
            let gid = Self::id_for(tx, &mut self.genres, "genres", g)?;
            tx.prepare_cached(
                "INSERT OR IGNORE INTO release_genres(release_id, genre_id) VALUES (?1, ?2)",
            )?
            .execute(params![id, gid])?;
        }
        for s in &r.styles {
            let sid = Self::id_for(tx, &mut self.styles, "styles", s)?;
            tx.prepare_cached(
                "INSERT OR IGNORE INTO release_styles(release_id, style_id) VALUES (?1, ?2)",
            )?
            .execute(params![id, sid])?;
        }
        for (i, t) in r.tracks.iter().enumerate() {
            tx.prepare_cached(
                "INSERT OR REPLACE INTO tracks(release_id, position, pos, title, duration)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![id, i as i64, t.position, t.title, t.duration])?;
        }
        for (i, v) in r.videos.iter().enumerate() {
            tx.prepare_cached(
                "INSERT OR REPLACE INTO videos(release_id, position, url, title, duration)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![id, i as i64, v.url, v.title, v.duration])?;
        }
        for (i, c) in r.credits.iter().enumerate() {
            tx.prepare_cached(
                "INSERT OR REPLACE INTO credits(release_id, position, artist_id, name, role, tracks)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![id, i as i64, c.id as i64, c.name, c.role, c.tracks])?;
        }

        let track_titles = r
            .tracks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut artists_text = artist_line;
        for a in &r.artists {
            artists_text.push(' ');
            artists_text.push_str(&a.name);
        }
        tx.prepare_cached(
            "INSERT INTO releases_fts(rowid, artists, title, labels, catnos, tracks)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            id,
            artists_text,
            r.title,
            label_line,
            catno_line,
            track_titles
        ])?;
        Ok(())
    }
}

/// Une valores no vacíos sin repetir, en orden.
fn dedup_join<'a>(items: impl Iterator<Item = &'a str>, sep: &str) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for s in items {
        if !s.is_empty() && !seen.contains(&s) {
            seen.push(s);
        }
    }
    seen.join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discogs::Format;

    fn rel(fmt: &str, genres: &[&str], styles: &[&str], year: &str) -> Release {
        Release {
            formats: vec![Format {
                name: fmt.into(),
                ..Default::default()
            }],
            genres: genres.iter().map(|s| (*s).into()).collect(),
            styles: styles.iter().map(|s| (*s).into()).collect(),
            released: year.into(),
            ..Default::default()
        }
    }

    #[test]
    fn filtros() {
        let th = rel("Vinyl", &["Electronic"], &["Tech House"], "1998");
        let rock = rel("Vinyl", &["Rock"], &["Experimental"], "1981");
        let cd = rel("CD", &["Electronic"], &["Tech House"], "2001");
        let sin_anyo = rel("Vinyl", &["Electronic"], &["Ambient"], "");

        let todo = ImportFilter::default();
        assert!([&th, &rock, &cd, &sin_anyo].iter().all(|r| todo.accepts(r)));

        let vinilo = ImportFilter::all_vinyl();
        assert!(vinilo.accepts(&th) && vinilo.accepts(&rock) && !vinilo.accepts(&cd));

        let solo_th = ImportFilter {
            styles: vec![("Electronic".into(), "Tech House".into())],
            ..Default::default()
        };
        assert!(solo_th.accepts(&th) && solo_th.accepts(&cd));
        assert!(!solo_th.accepts(&sin_anyo) && !solo_th.accepts(&rock));

        // El estilo se elige dentro de su género: Electronic → Experimental no trae Rock.
        let exp = ImportFilter {
            styles: vec![("Electronic".into(), "Experimental".into())],
            ..Default::default()
        };
        assert!(!exp.accepts(&rock));

        let mezcla = ImportFilter {
            genres: vec!["Rock".into()],
            styles: vec![("Electronic".into(), "Tech House".into())],
            year_from: Some(1990),
            year_to: Some(2026),
            formats: vec!["Vinyl".into()],
        };
        assert!(mezcla.accepts(&th));
        assert!(!mezcla.accepts(&rock), "1981 fuera de rango");
        assert!(!mezcla.accepts(&sin_anyo));
        assert_eq!(
            mezcla.describe(),
            "Vinyl · Rock, Electronic → Tech House · 1990–2026"
        );
        assert_eq!(ImportFilter::default().describe(), "Todo");
    }

    #[test]
    fn fecha_del_dump_y_hora() {
        assert_eq!(
            dump_date(Path::new("/x/discogs_20261101_releases.xml.gz")).as_deref(),
            Some("2026-11-01")
        );
        assert_eq!(dump_date(Path::new("muestra.xml")), None);
        let n = now_iso();
        assert_eq!(n.len(), 20);
        assert!(n.starts_with("20"));
    }
}
