//! Configuración de la interfaz en `~/.config/atenea/config.toml` (T02): por ahora, el tema.
//! Se escribe de forma atómica; si falta o está dañada, valores por defecto.
//!
//! Basado en la configuración de Digityzer (`crates/digityzer-app/src/ui/config.rs`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::theme::Theme;

/// Configuración guardada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Identificador del tema (`dracula`, `nord`…).
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default_theme().id.into(),
        }
    }
}

/// Ruta del archivo: `$XDG_CONFIG_HOME/atenea/config.toml` o `~/.config/atenea/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("atenea").join("config.toml"))
}

impl Config {
    /// Lee la configuración de `path`; si no existe o no se entiende, la de por defecto.
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("configuración ilegible en {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Lee la configuración del usuario.
    pub fn load() -> Self {
        config_path().map_or_else(Self::default, |p| Self::load_from(&p))
    }

    /// Guarda en `path` de forma atómica.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(path, text.as_bytes())
    }

    /// Guarda la configuración del usuario; los errores solo se registran.
    pub fn save(&self) {
        if let Some(p) = config_path()
            && let Err(e) = self.save_to(&p)
        {
            tracing::warn!("no se pudo guardar {}: {e}", p.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_y_vuelta_y_valores_por_defecto() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("atenea").join("config.toml");
        assert_eq!(Config::load_from(&p), Config::default());
        let c = Config {
            theme: "nord".into(),
        };
        c.save_to(&p).unwrap();
        assert_eq!(Config::load_from(&p), c);
        std::fs::write(&p, "theme = 3\n").unwrap();
        assert_eq!(Config::load_from(&p), Config::default());
        std::fs::write(&p, "theme = \"gruvbox-dark\"\n").unwrap();
        assert_eq!(Config::load_from(&p).theme, "gruvbox-dark");
    }
}
