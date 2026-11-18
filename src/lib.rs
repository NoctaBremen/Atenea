//! Vault: explorador de vinilos sobre los dumps de Discogs.
//!
//! La lógica vive en la biblioteca y se prueba sin GTK; el binario `vault` es la interfaz.

pub mod catalog;
pub mod config;
pub mod db;
pub mod discogs;
pub mod export;
pub mod fsutil;
pub mod player;
pub mod theme;
pub mod userdb;
