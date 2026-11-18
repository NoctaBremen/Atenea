//! Acceso de lectura a una base importada (T08–T13): información, ids filtrados, filas por
//! páginas y detalle de un disco.
//!
//! La base de Discogs se abre en solo lectura (`mode=ro`) y la de usuario va adjunta como `u`
//! en lectura y escritura.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params_from_iter};

use super::query::{Filtros, Sort, ids_query};
use crate::userdb::{self, UserData};

/// Datos de cabecera de una base.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseInfo {
    /// Ruta del archivo.
    pub path: PathBuf,
    /// Nombre («Todo en vinilo»).
    pub name: String,
    /// Fecha del dump (`2026-11-01`), vacía si no se sabe.
    pub dump_date: String,
    /// Nº de discos.
    pub release_count: i64,
    /// Filtro de importación, en texto.
    pub filter_text: String,
    /// Fecha de importación.
    pub imported_at: String,
}

/// Una fila de la lista.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    /// Id de Discogs.
    pub id: i64,
    /// Artistas.
    pub artist: String,
    /// Título.
    pub title: String,
    /// Sellos.
    pub label: String,
    /// Números de catálogo.
    pub catno: String,
    /// Año.
    pub year: Option<i64>,
    /// País.
    pub country: String,
    /// Estilos.
    pub styles: String,
    /// Formato en texto.
    pub format: String,
    /// Nº de vídeos.
    pub n_videos: i64,
}

/// Pista del detalle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailTrack {
    /// Posición.
    pub pos: String,
    /// Título.
    pub title: String,
    /// Duración.
    pub duration: String,
}

/// Vídeo del detalle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailVideo {
    /// URL.
    pub url: String,
    /// Título.
    pub title: String,
    /// Duración en segundos.
    pub duration: i64,
}

/// Detalle de un disco (T13).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detail {
    /// Fila de la lista.
    pub row: Row,
    /// Fecha de publicación tal como viene.
    pub released: String,
    /// Géneros.
    pub genres: String,
    /// Sellos con su catno: («Sello», «CAT 001»).
    pub labels: Vec<(String, String)>,
    /// Pistas.
    pub tracks: Vec<DetailTrack>,
    /// Créditos: («Papel», «Nombre»).
    pub credits: Vec<(String, String)>,
    /// Notas de Discogs.
    pub notes: String,
    /// Vídeos.
    pub videos: Vec<DetailVideo>,
    /// Id del master.
    pub master_id: Option<i64>,
}

impl Detail {
    /// Enlace a la página del disco en Discogs.
    pub fn discogs_url(&self) -> String {
        discogs_url(self.row.id)
    }
}

/// Enlace a la página de un disco en Discogs.
pub fn discogs_url(id: i64) -> String {
    format!("https://www.discogs.com/release/{id}")
}

/// Descripciones de formato que ofrece el panel de filtros.
pub const FORMAT_DESCRIPTIONS: [&str; 8] = [
    "LP",
    "12\"",
    "10\"",
    "7\"",
    "EP",
    "Single",
    "Album",
    "Compilation",
];

/// Una base abierta.
pub struct Base {
    conn: Connection,
    info: BaseInfo,
}

/// Error al abrir una base.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// SQLite.
    #[error("no se puede abrir la base: {0}")]
    Sql(#[from] rusqlite::Error),
    /// No es una base de Atenea.
    #[error("{0} no es una base de Atenea")]
    NotABase(String),
}

const ROW_COLS: &str = "id, artist_line, title, label_line, catno_line, year, country, style_line, format_line, n_videos";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        artist: r.get(1)?,
        title: r.get(2)?,
        label: r.get(3)?,
        catno: r.get(4)?,
        year: r.get(5)?,
        country: r.get(6)?,
        styles: r.get(7)?,
        format: r.get(8)?,
        n_videos: r.get(9)?,
    })
}

impl Base {
    /// Abre la base de Discogs de `path` en solo lectura y adjunta la de usuario (`None` = una
    /// en memoria, para tests).
    pub fn open(path: &Path, user_db: Option<&Path>) -> Result<Self, OpenError> {
        if !path.is_file() {
            return Err(OpenError::NotABase(path.display().to_string()));
        }
        let uri = format!(
            "file:{}?mode=ro",
            path.to_string_lossy()
                .replace('%', "%25")
                .replace('?', "%3f")
                .replace('#', "%23")
        );
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA cache_size = -131072; PRAGMA temp_store = MEMORY;")?;
        let has_releases: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'releases'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| OpenError::NotABase(path.display().to_string()))?;
        if has_releases.is_none() {
            return Err(OpenError::NotABase(path.display().to_string()));
        }
        match user_db {
            Some(p) => userdb::attach(&conn, p)?,
            None => userdb::attach_in_memory(&conn)?,
        }
        let meta = |k: &str| -> String {
            conn.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get(0))
                .unwrap_or_default()
        };
        let mut info = BaseInfo {
            path: path.to_owned(),
            name: meta("name"),
            dump_date: meta("dump_date"),
            release_count: meta("release_count").parse().unwrap_or(0),
            filter_text: meta("filter_text"),
            imported_at: meta("imported_at"),
        };
        if info.name.is_empty() {
            info.name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        Ok(Self { conn, info })
    }

    /// Información de la base.
    pub fn info(&self) -> &BaseInfo {
        &self.info
    }

    /// Tus datos (marcadores, notas, maletas…).
    pub fn user(&self) -> UserData<'_> {
        UserData::new(&self.conn)
    }

    /// Asa para interrumpir una consulta larga desde otro hilo.
    pub fn interrupt_handle(&self) -> rusqlite::InterruptHandle {
        self.conn.get_interrupt_handle()
    }

    /// Ids que cumplen los filtros, en orden.
    pub fn ids(&self, f: &Filtros, sort: Sort, limit: Option<usize>) -> rusqlite::Result<Vec<i64>> {
        let q = ids_query(f, sort, limit);
        let mut st = self.conn.prepare_cached(&q.sql)?;
        let rows = st.query_map(params_from_iter(q.params.iter()), |r| r.get(0))?;
        rows.collect()
    }

    /// Filas de estos ids, en el mismo orden (los ids que no existan se omiten).
    pub fn rows(&self, ids: &[i64]) -> rusqlite::Result<Vec<Row>> {
        let mut out = Vec::with_capacity(ids.len());
        // Por trozos para no pasar del límite de parámetros de SQLite.
        for chunk in ids.chunks(500) {
            let sql = format!(
                "SELECT {ROW_COLS} FROM releases WHERE id IN ({})",
                vec!["?"; chunk.len()].join(",")
            );
            let mut st = self.conn.prepare(&sql)?;
            let mut got: std::collections::HashMap<i64, Row> = st
                .query_map(params_from_iter(chunk), read_row)?
                .map(|r| r.map(|row| (row.id, row)))
                .collect::<rusqlite::Result<_>>()?;
            out.extend(chunk.iter().filter_map(|id| got.remove(id)));
        }
        Ok(out)
    }

    /// Detalle de un disco.
    pub fn detail(&self, id: i64) -> rusqlite::Result<Option<Detail>> {
        let row = self
            .conn
            .query_row(
                &format!("SELECT {ROW_COLS} FROM releases WHERE id = ?1"),
                [id],
                read_row,
            )
            .optional()?;
        let Some(row) = row else {
            return Ok(None);
        };
        let (released, genres, notes, master_id): (String, String, String, Option<i64>) =
            self.conn.query_row(
                "SELECT released, genre_line, notes, master_id FROM releases WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        let labels = self
            .conn
            .prepare_cached(
                "SELECT l.name, rl.catno FROM release_labels rl JOIN labels l ON l.id = rl.label_id
                 WHERE rl.release_id = ?1 ORDER BY rl.position",
            )?
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let tracks = self
            .conn
            .prepare_cached(
                "SELECT pos, title, duration FROM tracks WHERE release_id = ?1 ORDER BY position",
            )?
            .query_map([id], |r| {
                Ok(DetailTrack {
                    pos: r.get(0)?,
                    title: r.get(1)?,
                    duration: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let credits = self
            .conn
            .prepare_cached(
                "SELECT role, name FROM credits WHERE release_id = ?1 ORDER BY position",
            )?
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let videos = self
            .conn
            .prepare_cached(
                "SELECT url, title, duration FROM videos WHERE release_id = ?1 ORDER BY position",
            )?
            .query_map([id], |r| {
                Ok(DetailVideo {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    duration: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(Detail {
            row,
            released,
            genres,
            labels,
            tracks,
            credits,
            notes,
            videos,
            master_id,
        }))
    }

    fn names(&self, sql: &str) -> rusqlite::Result<Vec<(String, i64)>> {
        self.conn
            .prepare(sql)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }

    /// Géneros presentes con su nº de discos, por nombre.
    pub fn genres(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        self.names(
            "SELECT g.name, count(*) FROM release_genres rg JOIN genres g ON g.id = rg.genre_id
             GROUP BY g.id ORDER BY g.name",
        )
    }

    /// Estilos presentes con su nº de discos, por nombre.
    pub fn styles(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        self.names(
            "SELECT s.name, count(*) FROM release_styles rs JOIN styles s ON s.id = rs.style_id
             GROUP BY s.id ORDER BY s.name",
        )
    }

    /// Países presentes con su nº de discos, de más a menos.
    pub fn countries(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        self.names(
            "SELECT country, count(*) AS n FROM releases WHERE country <> ''
             GROUP BY country ORDER BY n DESC, country",
        )
    }
}
