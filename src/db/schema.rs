//! Esquema de la base de Discogs (T05).
//!
//! La base se importa una vez y después solo se lee. Las tablas de relación guardan el detalle
//! (artistas, sellos, formatos, géneros, estilos, pistas, vídeos, créditos) y `releases` lleva
//! además columnas ya compuestas (`artist_line`, `label_line`…) para que la lista se pinte sin
//! joins. El índice de texto es FTS5 sin contenido (`content=''`): solo guarda el índice, con
//! `rowid` = id del disco.

use rusqlite::Connection;

/// Versión del esquema (se guarda en `PRAGMA user_version`).
pub const SCHEMA_VERSION: i32 = 2;

/// Descripciones de formato que se pueden filtrar, cada una con su bit en `releases.fmt_flags`
/// (el filtro es un AND de bits sobre la fila, sin subconsultas).
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

/// Bits de formato de un disco a partir de sus descripciones.
pub fn format_flags<'a>(descriptions: impl IntoIterator<Item = &'a str>) -> i64 {
    let mut f = 0;
    for d in descriptions {
        if let Some(i) = FORMAT_DESCRIPTIONS
            .iter()
            .position(|x| x.eq_ignore_ascii_case(d.trim()))
        {
            f |= 1 << i;
        }
    }
    f
}

/// Máscara de bits de unas descripciones elegidas en el filtro.
pub fn format_mask(descriptions: &[String]) -> i64 {
    format_flags(descriptions.iter().map(String::as_str))
}

/// Tablas (sin los índices secundarios, que se crean al final de la importación).
pub const TABLES: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE releases (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    artist_line TEXT NOT NULL,
    label_line  TEXT NOT NULL,
    catno_line  TEXT NOT NULL,
    format_line TEXT NOT NULL,
    genre_line  TEXT NOT NULL,
    style_line  TEXT NOT NULL,
    country     TEXT NOT NULL,
    released    TEXT NOT NULL,
    year        INTEGER,
    notes       TEXT NOT NULL,
    master_id   INTEGER,
    n_videos    INTEGER NOT NULL,
    fmt_flags   INTEGER NOT NULL
);
CREATE TABLE artists (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE release_artists (
    release_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    artist_id  INTEGER NOT NULL,
    anv        TEXT NOT NULL,
    join_text  TEXT NOT NULL,
    PRIMARY KEY (release_id, position)
) WITHOUT ROWID;
CREATE TABLE labels (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE release_labels (
    release_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    label_id   INTEGER NOT NULL,
    catno      TEXT NOT NULL,
    PRIMARY KEY (release_id, position)
) WITHOUT ROWID;
CREATE TABLE formats (
    release_id   INTEGER NOT NULL,
    position     INTEGER NOT NULL,
    name         TEXT NOT NULL,
    qty          INTEGER NOT NULL,
    text         TEXT NOT NULL,
    descriptions TEXT NOT NULL, -- separadas por '|'
    PRIMARY KEY (release_id, position)
) WITHOUT ROWID;
CREATE TABLE genres (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE release_genres (
    release_id INTEGER NOT NULL,
    genre_id   INTEGER NOT NULL,
    PRIMARY KEY (release_id, genre_id)
) WITHOUT ROWID;
CREATE TABLE styles (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE release_styles (
    release_id INTEGER NOT NULL,
    style_id   INTEGER NOT NULL,
    PRIMARY KEY (release_id, style_id)
) WITHOUT ROWID;
CREATE TABLE tracks (
    release_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    pos        TEXT NOT NULL,
    title      TEXT NOT NULL,
    duration   TEXT NOT NULL,
    PRIMARY KEY (release_id, position)
) WITHOUT ROWID;
CREATE TABLE videos (
    release_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    url        TEXT NOT NULL,
    title      TEXT NOT NULL,
    duration   INTEGER NOT NULL,
    PRIMARY KEY (release_id, position)
) WITHOUT ROWID;
CREATE TABLE credits (
    release_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    artist_id  INTEGER NOT NULL,
    name       TEXT NOT NULL,
    role       TEXT NOT NULL,
    tracks     TEXT NOT NULL,
    PRIMARY KEY (release_id, position)
) WITHOUT ROWID;
-- Recuentos para el panel de filtros (género, estilo, país), calculados al importar: así
-- abrir una base no recorre millones de filas en el hilo de la interfaz.
CREATE TABLE facets (
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    n    INTEGER NOT NULL,
    PRIMARY KEY (kind, name)
) WITHOUT ROWID;
CREATE VIRTUAL TABLE releases_fts USING fts5(
    artists, title, labels, catnos, tracks,
    content = '',
    tokenize = 'unicode61 remove_diacritics 2'
);
";

/// Índices secundarios: se crean después de la carga masiva, que así va mucho más rápida.
pub const INDEXES: &str = "
-- Los índices de orden cubren el ORDER BY completo de cada columna de la lista (incluido el
-- desempate por id), para que la lista entera salga ordenada sin ordenar en memoria.
-- Año: un índice por sentido, con los discos sin año siempre al final.
CREATE INDEX idx_releases_year_asc  ON releases(coalesce(year, 99999), id);
CREATE INDEX idx_releases_year_desc ON releases(coalesce(year, -1), id);
CREATE INDEX idx_releases_country ON releases(country COLLATE NOCASE, id);
CREATE INDEX idx_releases_artist  ON releases(artist_line COLLATE NOCASE, title COLLATE NOCASE, id);
CREATE INDEX idx_releases_title   ON releases(title COLLATE NOCASE, id);
CREATE INDEX idx_releases_label   ON releases(label_line COLLATE NOCASE, catno_line COLLATE NOCASE, id);
CREATE INDEX idx_releases_catno   ON releases(catno_line COLLATE NOCASE, id);
CREATE INDEX idx_release_artists_artist ON release_artists(artist_id);
CREATE INDEX idx_release_labels_label   ON release_labels(label_id);
CREATE INDEX idx_release_genres_genre   ON release_genres(genre_id, release_id);
CREATE INDEX idx_release_styles_style   ON release_styles(style_id, release_id);
CREATE INDEX idx_formats_name           ON formats(name, release_id);
CREATE INDEX idx_artists_name ON artists(name COLLATE NOCASE);
CREATE INDEX idx_labels_name  ON labels(name COLLATE NOCASE);
";

/// Crea las tablas en una base vacía.
pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(TABLES)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
}

/// Rellena los recuentos del panel de filtros.
pub const FACETS: &str = "
DELETE FROM facets;
INSERT INTO facets(kind, name, n)
    SELECT 'genre', g.name, count(*) FROM release_genres rg JOIN genres g ON g.id = rg.genre_id
    GROUP BY g.id;
INSERT INTO facets(kind, name, n)
    SELECT 'style', s.name, count(*) FROM release_styles rs JOIN styles s ON s.id = rs.style_id
    GROUP BY s.id;
INSERT INTO facets(kind, name, n)
    SELECT 'country', country, count(*) FROM releases WHERE country <> '' GROUP BY country;
";

/// Crea los índices secundarios, los recuentos y actualiza las estadísticas del planificador.
pub fn create_indexes(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(INDEXES)?;
    conn.execute_batch(FACETS)?;
    conn.execute_batch("ANALYZE;")
}

/// Crea una base completa (tablas + índices) vacía. Útil en tests.
pub fn create_empty(conn: &Connection) -> rusqlite::Result<()> {
    create_tables(conn)?;
    create_indexes(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_de_formato() {
        assert_eq!(format_flags(["LP", "Album"]), 1 | (1 << 6));
        assert_eq!(
            format_flags(["12\"", "33 ⅓ RPM", "ep"]),
            (1 << 1) | (1 << 4)
        );
        assert_eq!(format_flags(["Test Pressing"]), 0);
        assert_eq!(format_mask(&["7\"".into()]), 1 << 3);
    }

    #[test]
    fn crea_base_vacia_con_indices() {
        let c = Connection::open_in_memory().unwrap();
        create_empty(&c).unwrap();
        let v: i32 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 14);
        // FTS5 disponible (SQLite incluido con rusqlite).
        c.execute(
            "INSERT INTO releases_fts(rowid, artists, title, labels, catnos, tracks)
             VALUES (1, 'Kalo Tren', 'Axis', 'Ondas', 'OND 001', '')",
            [],
        )
        .unwrap();
        let id: i64 = c
            .query_row(
                "SELECT rowid FROM releases_fts WHERE releases_fts MATCH 'kalo AND axis'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id, 1);
    }
}
