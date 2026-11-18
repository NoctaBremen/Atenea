//! Atenea: explorador de vinilos sobre los dumps de Discogs.
//!
//! La lógica vive en la biblioteca y se prueba sin GTK; el binario `atenea` es la interfaz.

pub mod config;
pub mod discogs;
pub mod fsutil;
pub mod theme;
