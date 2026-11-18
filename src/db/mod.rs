//! Base de Discogs en SQLite: esquema, importación y consultas.

pub mod base;
pub mod import;
pub mod query;
pub mod schema;

pub use base::{Base, BaseInfo, Detail, OpenError, Row};
pub use import::{ImportError, ImportFilter, ImportOptions, ImportStats, Progress, import};
pub use query::{Filtros, Mark, Sort, SortKey};
