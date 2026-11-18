//! Tus datos (T18–T22, T14): marcadores, notas, maletas y búsquedas guardadas, en
//! `~/.local/share/vault/user.db`.
//!
//! Va aparte de la base de Discogs y se enlaza solo por `release_id`: la base de Discogs se abre
//! en solo lectura y se puede sustituir (o tener varias) sin perder nada de esto.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::query::{Filtros, Mark, Sort};

/// Versión del esquema de usuario.
pub const USER_SCHEMA_VERSION: i32 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS marks (
    release_id INTEGER PRIMARY KEY,
    state      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_marks_state ON marks(state);
CREATE TABLE IF NOT EXISTS notes (
    release_id INTEGER PRIMARY KEY,
    text       TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS suitcases (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    color      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS suitcase_items (
    suitcase_id INTEGER NOT NULL REFERENCES suitcases(id) ON DELETE CASCADE,
    release_id  INTEGER NOT NULL,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (suitcase_id, release_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_suitcase_items_release ON suitcase_items(release_id);
CREATE TABLE IF NOT EXISTS saved_searches (
    id      INTEGER PRIMARY KEY,
    name    TEXT NOT NULL UNIQUE,
    filtros TEXT NOT NULL,
    sort    TEXT NOT NULL
);
";

/// Colores por defecto de las maletas nuevas (roles del tema, ver [`crate::theme`]).
pub const SUITCASE_COLORS: [&str; 6] = ["curva", "media", "picos", "ok", "aviso", "clip"];

/// Una maleta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suitcase {
    /// Id.
    pub id: i64,
    /// Nombre.
    pub name: String,
    /// Color: nombre de un rol del tema (`curva`, `ok`…).
    pub color: String,
    /// Nº de discos.
    pub count: i64,
}

/// Búsqueda guardada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    /// Id.
    pub id: i64,
    /// Nombre.
    pub name: String,
    /// Filtros.
    pub filtros: Filtros,
    /// Orden.
    pub sort: Sort,
}

/// Error de la base de usuario.
#[derive(Debug, thiserror::Error)]
pub enum UserDbError {
    /// SQLite.
    #[error("error en la base de usuario: {0}")]
    Sql(#[from] rusqlite::Error),
    /// JSON de una búsqueda guardada.
    #[error("búsqueda guardada ilegible: {0}")]
    Json(#[from] serde_json::Error),
    /// Nombre vacío o repetido.
    #[error("{0}")]
    Invalid(String),
}

type Result<T> = std::result::Result<T, UserDbError>;

/// Ruta por defecto: `$XDG_DATA_HOME/vault/user.db` o `~/.local/share/vault/user.db`.
pub fn default_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("user.db"))
}

/// Directorio de datos de Vault (`~/.local/share/vault`).
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
        })?;
    Some(base.join("vault"))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Prepara una conexión (propia o adjunta como `schema`) con las tablas de usuario.
pub fn init_schema(conn: &Connection, schema: &str) -> rusqlite::Result<()> {
    let sql = SCHEMA
        .replace("EXISTS marks", &format!("EXISTS {schema}.marks"))
        .replace("EXISTS idx_", &format!("EXISTS {schema}.idx_"))
        .replace("EXISTS notes", &format!("EXISTS {schema}.notes"))
        .replace("EXISTS suitcases", &format!("EXISTS {schema}.suitcases"))
        .replace(
            "EXISTS suitcase_items",
            &format!("EXISTS {schema}.suitcase_items"),
        )
        .replace(
            "EXISTS saved_searches",
            &format!("EXISTS {schema}.saved_searches"),
        );
    conn.execute_batch(&sql)?;
    conn.pragma_update(Some(schema), "user_version", USER_SCHEMA_VERSION)
}

/// Acceso a la base de usuario a través de una conexión en la que está adjunta como `u`.
///
/// No posee la conexión: la usa [`crate::db::Base`], que tiene la de Discogs con `u` adjunta.
pub struct UserData<'c> {
    conn: &'c Connection,
}

impl<'c> UserData<'c> {
    /// Sobre una conexión con la base de usuario adjunta como `u`.
    pub fn new(conn: &'c Connection) -> Self {
        Self { conn }
    }

    // --- Marcadores (T19) ---

    /// Marca un disco (o quita el marcador con `None`).
    pub fn set_mark(&self, release_id: i64, mark: Option<Mark>) -> Result<()> {
        match mark {
            Some(m) => self.conn.execute(
                "INSERT INTO u.marks(release_id, state, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(release_id) DO UPDATE SET state = ?2, updated_at = ?3",
                params![release_id, m.key(), now()],
            )?,
            None => self
                .conn
                .execute("DELETE FROM u.marks WHERE release_id = ?1", [release_id])?,
        };
        Ok(())
    }

    /// Marcador de un disco.
    pub fn mark(&self, release_id: i64) -> Result<Option<Mark>> {
        let k: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM u.marks WHERE release_id = ?1",
                [release_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(k.as_deref().and_then(Mark::from_key))
    }

    /// Marcadores de varios discos (para pintar una página de la lista).
    pub fn marks_for(&self, ids: &[i64]) -> Result<Vec<(i64, Mark)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT release_id, state FROM u.marks WHERE release_id IN ({})",
            vec!["?"; ids.len()].join(",")
        );
        let mut st = self.conn.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(ids), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, k) = row?;
            if let Some(m) = Mark::from_key(&k) {
                out.push((id, m));
            }
        }
        Ok(out)
    }

    // --- Notas (T22) ---

    /// Guarda la nota de un disco (vacía = borrarla).
    pub fn set_note(&self, release_id: i64, text: &str) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            self.conn
                .execute("DELETE FROM u.notes WHERE release_id = ?1", [release_id])?;
        } else {
            self.conn.execute(
                "INSERT INTO u.notes(release_id, text, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(release_id) DO UPDATE SET text = ?2, updated_at = ?3",
                params![release_id, text, now()],
            )?;
        }
        Ok(())
    }

    /// Nota de un disco (vacía si no tiene).
    pub fn note(&self, release_id: i64) -> Result<String> {
        Ok(self
            .conn
            .query_row(
                "SELECT text FROM u.notes WHERE release_id = ?1",
                [release_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    // --- Maletas (T20, T21) ---

    /// Crea una maleta. El nombre no puede estar vacío ni repetido.
    pub fn create_suitcase(&self, name: &str, color: &str) -> Result<i64> {
        let name = self.valid_suitcase_name(name, None)?;
        self.conn.execute(
            "INSERT INTO u.suitcases(name, color, created_at) VALUES (?1, ?2, ?3)",
            params![name, color, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn valid_suitcase_name(&self, name: &str, except: Option<i64>) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(UserDbError::Invalid("La maleta necesita un nombre.".into()));
        }
        let clash: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM u.suitcases WHERE name = ?1 COLLATE NOCASE",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if clash.is_some_and(|id| Some(id) != except) {
            return Err(UserDbError::Invalid(format!("Ya hay una maleta «{name}».")));
        }
        Ok(name.to_owned())
    }

    /// Renombra una maleta.
    pub fn rename_suitcase(&self, id: i64, name: &str) -> Result<()> {
        let name = self.valid_suitcase_name(name, Some(id))?;
        self.conn.execute(
            "UPDATE u.suitcases SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// Cambia el color de una maleta.
    pub fn set_suitcase_color(&self, id: i64, color: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE u.suitcases SET color = ?2 WHERE id = ?1",
            params![id, color],
        )?;
        Ok(())
    }

    /// Borra una maleta (los discos no se tocan).
    pub fn delete_suitcase(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM u.suitcase_items WHERE suitcase_id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM u.suitcases WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Maletas con su nº de discos, por nombre.
    pub fn suitcases(&self) -> Result<Vec<Suitcase>> {
        let mut st = self.conn.prepare(
            "SELECT s.id, s.name, s.color,
                    (SELECT count(*) FROM u.suitcase_items i WHERE i.suitcase_id = s.id)
             FROM u.suitcases s ORDER BY s.name COLLATE NOCASE",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Suitcase {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
                count: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Mete un disco en una maleta (si ya estaba, no pasa nada).
    pub fn add_to_suitcase(&self, suitcase_id: i64, release_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO u.suitcase_items(suitcase_id, release_id, added_at)
             VALUES (?1, ?2, ?3)",
            params![suitcase_id, release_id, now()],
        )?;
        Ok(())
    }

    /// Saca un disco de una maleta.
    pub fn remove_from_suitcase(&self, suitcase_id: i64, release_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM u.suitcase_items WHERE suitcase_id = ?1 AND release_id = ?2",
            params![suitcase_id, release_id],
        )?;
        Ok(())
    }

    /// Maletas en las que está un disco.
    pub fn suitcases_of(&self, release_id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(
            "SELECT suitcase_id FROM u.suitcase_items WHERE release_id = ?1 ORDER BY suitcase_id",
        )?;
        let rows = st.query_map([release_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids de los discos de una maleta, en orden de llegada.
    pub fn suitcase_items(&self, suitcase_id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(
            "SELECT release_id FROM u.suitcase_items WHERE suitcase_id = ?1
             ORDER BY added_at, release_id",
        )?;
        let rows = st.query_map([suitcase_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // --- Búsquedas guardadas (T14) ---

    /// Guarda (o sobrescribe, si el nombre existe) una búsqueda.
    pub fn save_search(&self, name: &str, filtros: &Filtros, sort: Sort) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(UserDbError::Invalid(
                "La búsqueda necesita un nombre.".into(),
            ));
        }
        self.conn.execute(
            "INSERT INTO u.saved_searches(name, filtros, sort) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET filtros = ?2, sort = ?3",
            params![
                name,
                serde_json::to_string(filtros)?,
                serde_json::to_string(&sort)?
            ],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM u.saved_searches WHERE name = ?1",
            [name],
            |r| r.get(0),
        )?)
    }

    /// Renombra una búsqueda guardada.
    pub fn rename_search(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(UserDbError::Invalid(
                "La búsqueda necesita un nombre.".into(),
            ));
        }
        self.conn
            .execute(
                "UPDATE u.saved_searches SET name = ?2 WHERE id = ?1",
                params![id, name],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    UserDbError::Invalid(format!("Ya hay una búsqueda «{name}»."))
                }
                e => e.into(),
            })?;
        Ok(())
    }

    /// Borra una búsqueda guardada.
    pub fn delete_search(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM u.saved_searches WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Búsquedas guardadas, por nombre. Las ilegibles se saltan con un aviso.
    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>> {
        let mut st = self.conn.prepare(
            "SELECT id, name, filtros, sort FROM u.saved_searches ORDER BY name COLLATE NOCASE",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, f, s) = row?;
            match (serde_json::from_str(&f), serde_json::from_str(&s)) {
                (Ok(filtros), Ok(sort)) => out.push(SavedSearch {
                    id,
                    name,
                    filtros,
                    sort,
                }),
                _ => tracing::warn!("búsqueda guardada «{name}» ilegible; se omite"),
            }
        }
        Ok(out)
    }
}

/// Adjunta la base de usuario de `path` como `u` (creándola si no existe).
pub fn attach(conn: &Connection, path: &Path) -> rusqlite::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    conn.execute("ATTACH DATABASE ?1 AS u", [path.to_string_lossy().as_ref()])?;
    init_schema(conn, "u")
}

/// Adjunta una base de usuario vacía en memoria (tests y modo sin disco).
pub fn attach_in_memory(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("ATTACH DATABASE ':memory:' AS u")?;
    init_schema(conn, "u")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::query::SortKey;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        attach_in_memory(&c).unwrap();
        c
    }

    #[test]
    fn marcadores_y_notas() {
        let c = conn();
        let u = UserData::new(&c);
        u.set_mark(5, Some(Mark::Interesa)).unwrap();
        u.set_mark(5, Some(Mark::Volver)).unwrap();
        u.set_mark(6, Some(Mark::NoInteresa)).unwrap();
        assert_eq!(u.mark(5).unwrap(), Some(Mark::Volver));
        let mut m = u.marks_for(&[5, 6, 7]).unwrap();
        m.sort_by_key(|x| x.0);
        assert_eq!(m, vec![(5, Mark::Volver), (6, Mark::NoInteresa)]);
        u.set_mark(5, None).unwrap();
        assert_eq!(u.mark(5).unwrap(), None);

        u.set_note(5, "  buen B2 ").unwrap();
        assert_eq!(u.note(5).unwrap(), "buen B2");
        u.set_note(5, "").unwrap();
        assert_eq!(u.note(5).unwrap(), "");
    }

    #[test]
    fn maletas() {
        let c = conn();
        let u = UserData::new(&c);
        let th = u.create_suitcase("Techno House", "curva").unwrap();
        let rock = u.create_suitcase("Rock", "ok").unwrap();
        assert!(u.create_suitcase("rock", "ok").is_err(), "nombre repetido");
        assert!(u.create_suitcase("  ", "ok").is_err());
        u.add_to_suitcase(th, 1).unwrap();
        u.add_to_suitcase(th, 1).unwrap();
        u.add_to_suitcase(rock, 1).unwrap();
        u.add_to_suitcase(th, 2).unwrap();
        assert_eq!(u.suitcases_of(1).unwrap(), vec![th, rock]);
        assert_eq!(u.suitcase_items(th).unwrap(), vec![1, 2]);
        u.rename_suitcase(th, "Tech House").unwrap();
        u.rename_suitcase(th, "tech house").unwrap();
        assert!(u.rename_suitcase(th, "Rock").is_err());
        u.set_suitcase_color(rock, "clip").unwrap();
        let all = u.suitcases().unwrap();
        assert_eq!(all[0].name, "Rock");
        assert_eq!(all[0].color, "clip");
        assert_eq!(all[1].count, 2);
        u.remove_from_suitcase(th, 2).unwrap();
        u.delete_suitcase(rock).unwrap();
        assert_eq!(u.suitcases_of(1).unwrap(), vec![th]);
    }

    #[test]
    fn busquedas_guardadas() {
        let c = conn();
        let u = UserData::new(&c);
        let f = Filtros {
            texto: "axis".into(),
            marcadores: vec![Mark::Interesa],
            ..Default::default()
        };
        let s = Sort {
            key: SortKey::Anyo,
            desc: true,
        };
        let id = u.save_search("Mi búsqueda", &f, s).unwrap();
        assert_eq!(u.save_search("Mi búsqueda", &f, s).unwrap(), id);
        let other = u
            .save_search("Otra", &Filtros::default(), Sort::default())
            .unwrap();
        assert!(u.rename_search(other, "Mi búsqueda").is_err());
        u.rename_search(id, "Renombrada").unwrap();
        let all = u.saved_searches().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Otra");
        assert_eq!(all[1].filtros, f);
        assert_eq!(all[1].sort, s);
        u.delete_search(other).unwrap();
        assert_eq!(u.saved_searches().unwrap().len(), 1);
    }

    #[test]
    fn archivo_persistente() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("user.db");
        {
            let c = Connection::open_in_memory().unwrap();
            attach(&c, &p).unwrap();
            UserData::new(&c).set_note(9, "hola").unwrap();
        }
        let c = Connection::open_in_memory().unwrap();
        attach(&c, &p).unwrap();
        assert_eq!(UserData::new(&c).note(9).unwrap(), "hola");
    }
}
