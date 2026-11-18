//! Constructor de consultas (T09) y búsqueda de texto (T12).
//!
//! [`Filtros`] → SQL parametrizado que devuelve los ids de los discos que cumplen todos los
//! filtros (AND entre filtros; dentro de una multiselección, OR). La lista guarda solo los ids y
//! pide las filas por páginas ([`super::Base::rows`]).
//!
//! La base de usuario va siempre adjunta como `u` (ver [`crate::userdb`]): de ella salen los
//! filtros de marcador y maleta, y el texto libre busca también en tus notas.

use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

/// Estado de un marcador (T19). Se guarda como texto en la base de usuario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mark {
    /// Me interesa (tecla 1).
    Interesa,
    /// No me interesa (tecla 2).
    NoInteresa,
    /// Escuchado (tecla 3).
    Escuchado,
    /// Volver a escuchar (tecla 4).
    Volver,
}

impl Mark {
    /// Todos, en el orden de las teclas 1–4.
    pub const ALL: [Mark; 4] = [
        Mark::Interesa,
        Mark::NoInteresa,
        Mark::Escuchado,
        Mark::Volver,
    ];

    /// Clave en la base.
    pub fn key(self) -> &'static str {
        match self {
            Mark::Interesa => "interesa",
            Mark::NoInteresa => "no_interesa",
            Mark::Escuchado => "escuchado",
            Mark::Volver => "volver",
        }
    }

    /// Desde la clave de la base.
    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == k)
    }

    /// Nombre visible.
    pub fn label(self) -> &'static str {
        match self {
            Mark::Interesa => "Me interesa",
            Mark::NoInteresa => "No me interesa",
            Mark::Escuchado => "Escuchado",
            Mark::Volver => "Volver a escuchar",
        }
    }
}

/// Columna de orden de la lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortKey {
    /// Artista, luego título.
    #[default]
    Artista,
    /// Título.
    Titulo,
    /// Sello, luego número de catálogo.
    Sello,
    /// Número de catálogo.
    Catno,
    /// Año.
    Anyo,
    /// País.
    Pais,
    /// Id de Discogs (orden de alta).
    Id,
}

impl SortKey {
    fn sql(self) -> &'static str {
        match self {
            SortKey::Artista => {
                "{p}r.artist_line COLLATE NOCASE {d}, {p}r.title COLLATE NOCASE {d}"
            }
            SortKey::Titulo => "{p}r.title COLLATE NOCASE {d}",
            SortKey::Sello => {
                "{p}r.label_line COLLATE NOCASE {d}, {p}r.catno_line COLLATE NOCASE {d}"
            }
            SortKey::Catno => "{p}r.catno_line COLLATE NOCASE {d}",
            // Sin año al final en los dos sentidos (ver los índices de año del esquema).
            SortKey::Anyo => "{p}{y} {d}",
            SortKey::Pais => "{p}r.country COLLATE NOCASE {d}",
            SortKey::Id => "{p}r.id {d}",
        }
    }
}

/// Orden de la lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Sort {
    /// Columna.
    pub key: SortKey,
    /// Descendente.
    pub desc: bool,
}

/// Filtros de búsqueda. Todos opcionales y combinables (AND).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filtros {
    /// Texto libre (artista, título, sello, catno, pistas). Admite `"frase exacta"`; las
    /// palabras sueltas buscan por prefijo.
    pub texto: String,
    /// Artista (prefijo por palabra).
    pub artista: String,
    /// Sello (prefijo por palabra).
    pub sello: String,
    /// Año mínimo.
    pub anyo_desde: Option<u16>,
    /// Año máximo.
    pub anyo_hasta: Option<u16>,
    /// País exacto (vacío = todos).
    pub pais: String,
    /// Géneros (OR).
    pub generos: Vec<String>,
    /// Estilos (OR).
    pub estilos: Vec<String>,
    /// Descripciones de formato (OR): `LP`, `12"`, `7"`…
    pub formatos: Vec<String>,
    /// Solo discos con vídeos.
    pub solo_con_videos: bool,
    /// Solo discos con alguno de estos marcadores (vacío = todos).
    pub marcadores: Vec<Mark>,
    /// Ocultar los marcados «No me interesa».
    pub ocultar_no_interesa: bool,
    /// Solo los discos sin ningún marcador (lo que queda por clasificar).
    pub sin_marcar: bool,
    /// Solo los discos de esta maleta.
    pub maleta: Option<i64>,
}

/// Escapa un término para FTS5 como cadena entre comillas.
fn quote(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Divide una búsqueda en términos: frases entre comillas y palabras sueltas.
fn terms(input: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find(|c: char| !c.is_whitespace()) {
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix('"') {
            let end = after.find('"').unwrap_or(after.len());
            let phrase = after[..end].trim();
            if !phrase.is_empty() {
                out.push((phrase.to_owned(), true));
            }
            rest = after.get(end + 1..).unwrap_or("");
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push((rest[..end].to_owned(), false));
            rest = &rest[end..];
        }
    }
    out
}

/// Convierte lo que escribe el usuario en una expresión FTS5 segura: las frases van exactas y
/// las palabras sueltas por prefijo. Los caracteres que el tokenizador ignora (puntuación) no
/// rompen la consulta porque todo va entre comillas. Devuelve `None` si no queda nada.
pub fn fts_expr(input: &str) -> Option<String> {
    let parts: Vec<String> = terms(input)
        .into_iter()
        .filter_map(|(t, phrase)| {
            // Un término sin letras ni cifras no aporta (y un prefijo vacío fallaría).
            if !t.chars().any(char::is_alphanumeric) {
                return None;
            }
            Some(if phrase {
                quote(&t)
            } else {
                format!("{}*", quote(&t))
            })
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" AND "))
    }
}

/// Consulta construida: SQL y parámetros.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// SQL con parámetros `?`.
    pub sql: String,
    /// Valores de los parámetros, en orden.
    pub params: Vec<Value>,
}

/// Escapa `%`, `_` y `\` para usarlo literal en `LIKE … ESCAPE '\'`.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// Condiciones `WHERE` (sin la palabra) y sus parámetros. Vacío = sin filtro.
pub fn where_clause(f: &Filtros) -> (Vec<String>, Vec<Value>) {
    let mut conds = Vec::new();
    let mut ps: Vec<Value> = Vec::new();

    if let Some(e) = fts_expr(&f.texto) {
        // Texto libre: índice FTS de Discogs o tus notas (T22).
        conds.push(
            "(r.id IN (SELECT rowid FROM releases_fts WHERE releases_fts MATCH ?) \
             OR r.id IN (SELECT release_id FROM u.notes WHERE text LIKE ? ESCAPE '\\'))"
                .into(),
        );
        ps.push(Value::Text(e));
        ps.push(Value::Text(format!("%{}%", like_escape(f.texto.trim()))));
    }
    let mut fts = Vec::new();
    if let Some(e) = fts_expr(&f.artista) {
        fts.push(format!("artists : ({e})"));
    }
    if let Some(e) = fts_expr(&f.sello) {
        fts.push(format!("labels : ({e})"));
    }
    if !fts.is_empty() {
        conds.push("r.id IN (SELECT rowid FROM releases_fts WHERE releases_fts MATCH ?)".into());
        ps.push(Value::Text(fts.join(" AND ")));
    }
    if let Some(y) = f.anyo_desde {
        conds.push("r.year >= ?".into());
        ps.push(Value::Integer(i64::from(y)));
    }
    if let Some(y) = f.anyo_hasta {
        conds.push("r.year <= ?".into());
        ps.push(Value::Integer(i64::from(y)));
    }
    if !f.pais.is_empty() {
        conds.push("r.country = ? COLLATE NOCASE".into());
        ps.push(Value::Text(f.pais.clone()));
    }
    // Géneros y estilos como conjunto (`IN`): SQLite lo saca del índice de la tabla de
    // relación una sola vez, en vez de una subconsulta por cada disco.
    if !f.generos.is_empty() {
        conds.push(format!(
            "r.id IN (SELECT rg.release_id FROM release_genres rg JOIN genres g ON g.id = rg.genre_id
                      WHERE g.name IN ({}))",
            placeholders(f.generos.len())
        ));
        ps.extend(f.generos.iter().cloned().map(Value::Text));
    }
    if !f.estilos.is_empty() {
        conds.push(format!(
            "r.id IN (SELECT rs.release_id FROM release_styles rs JOIN styles s ON s.id = rs.style_id
                      WHERE s.name IN ({}))",
            placeholders(f.estilos.len())
        ));
        ps.extend(f.estilos.iter().cloned().map(Value::Text));
    }
    let mask = super::schema::format_mask(&f.formatos);
    if mask != 0 {
        conds.push("(r.fmt_flags & ?) <> 0".into());
        ps.push(Value::Integer(mask));
    }
    if f.solo_con_videos {
        conds.push("r.n_videos > 0".into());
    }
    if !f.marcadores.is_empty() {
        conds.push(format!(
            "EXISTS (SELECT 1 FROM u.marks m WHERE m.release_id = r.id AND m.state IN ({}))",
            placeholders(f.marcadores.len())
        ));
        ps.extend(f.marcadores.iter().map(|m| Value::Text(m.key().to_owned())));
    }
    if f.ocultar_no_interesa {
        conds.push(
            "NOT EXISTS (SELECT 1 FROM u.marks m WHERE m.release_id = r.id AND m.state = ?)".into(),
        );
        ps.push(Value::Text(Mark::NoInteresa.key().into()));
    }
    if f.sin_marcar {
        conds.push("NOT EXISTS (SELECT 1 FROM u.marks m WHERE m.release_id = r.id)".into());
    }
    if let Some(id) = f.maleta {
        conds
            .push("r.id IN (SELECT release_id FROM u.suitcase_items WHERE suitcase_id = ?)".into());
        ps.push(Value::Integer(id));
    }
    (conds, ps)
}

/// Consulta de ids ordenados que cumplen los filtros, con límite opcional.
pub fn ids_query(f: &Filtros, sort: Sort, limit: Option<usize>) -> Query {
    let (conds, mut params) = where_clause(f);
    let mut sql = String::from("SELECT r.id FROM releases r");
    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }
    let dir = if sort.desc { "DESC" } else { "ASC" };
    sql.push_str(" ORDER BY ");
    let year = if sort.desc {
        "coalesce(r.year, -1)"
    } else {
        "coalesce(r.year, 99999)"
    };
    // Estrategia de orden. Sin filtros (o solo «ocultar No me interesa», que deja casi todo),
    // recorrer el índice de la columna da la lista ya ordenada. Con cualquier otro filtro sale
    // mucho más barato filtrar y ordenar el resultado: recorrer el índice comprobando cada fila
    // con acceso aleatorio cuesta lo mismo que recorrer toda la base (medido con un millón de
    // discos: 1,8 s contra 0,16 s para 65 000 resultados). El `+` unario impide que SQLite use
    // el índice para el `ORDER BY`.
    // «Ocultar No me interesa» y «Sin marcar» quitan pocos discos (lo marcado es una parte
    // pequeña de la base): solos, también dejan casi todo.
    let near_all_conds = usize::from(f.ocultar_no_interesa) + usize::from(f.sin_marcar);
    let nearly_all = conds.len() == near_all_conds;
    let prefix = if nearly_all { "" } else { "+" };
    sql.push_str(
        &sort
            .key
            .sql()
            .replace("{y}", year)
            .replace("{d}", dir)
            .replace("{p}", prefix),
    );
    sql.push_str(&format!(", {prefix}r.id {dir}"));
    if let Some(l) = limit {
        sql.push_str(" LIMIT ?");
        params.push(Value::Integer(l as i64));
    }
    Query { sql, params }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expresion_fts() {
        assert_eq!(
            fts_expr("\"jeff mills\" axis").as_deref(),
            Some("\"jeff mills\" AND \"axis\"*")
        );
        assert_eq!(fts_expr("  ").as_deref(), None);
        assert_eq!(fts_expr("- & \"\"").as_deref(), None);
        assert_eq!(
            fts_expr("a\"b \"sin cerrar").as_deref(),
            Some("\"a\"\"b\"* AND \"sin cerrar\"")
        );
        assert_eq!(fts_expr("AND OR").as_deref(), Some("\"AND\"* AND \"OR\"*"));
    }

    #[test]
    fn sin_filtros_ni_parametros() {
        let q = ids_query(&Filtros::default(), Sort::default(), None);
        assert!(!q.sql.contains("WHERE"));
        assert!(q.params.is_empty());
        assert!(q.sql.contains("ORDER BY r.artist_line"), "usa el índice");
        let solo_ocultar = Filtros {
            ocultar_no_interesa: true,
            ..Default::default()
        };
        assert!(
            ids_query(&solo_ocultar, Sort::default(), None)
                .sql
                .contains("ORDER BY r.artist_line")
        );
        let con_filtro = Filtros {
            solo_con_videos: true,
            ..Default::default()
        };
        let q = ids_query(&con_filtro, Sort::default(), None);
        assert!(q.sql.contains("ORDER BY +r.artist_line"), "{}", q.sql);
        assert!(q.sql.ends_with("+r.id ASC"));
    }

    #[test]
    fn parametros_en_orden() {
        let f = Filtros {
            texto: "axis".into(),
            anyo_desde: Some(1990),
            generos: vec!["Electronic".into(), "Rock".into()],
            formatos: vec!["12\"".into()],
            maleta: Some(3),
            ..Default::default()
        };
        let q = ids_query(
            &f,
            Sort {
                key: SortKey::Anyo,
                desc: true,
            },
            Some(10),
        );
        assert_eq!(q.sql.matches('?').count(), q.params.len());
        assert_eq!(q.params[0], Value::Text("\"axis\"*".into()));
        assert_eq!(q.params[1], Value::Text("%axis%".into()));
        assert_eq!(q.params[2], Value::Integer(1990));
        assert_eq!(q.params[5], Value::Integer(1 << 1));
        assert_eq!(q.params.last(), Some(&Value::Integer(10)));
        assert!(q.sql.contains("ORDER BY +coalesce(r.year, -1) DESC"));
    }

    #[test]
    fn marcadores() {
        for m in Mark::ALL {
            assert_eq!(Mark::from_key(m.key()), Some(m));
        }
        assert_eq!(Mark::from_key("x"), None);
    }
}
