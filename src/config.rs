//! Configuración en `~/.config/atenea/config.toml`: tema (T02), bases recientes (T08),
//! sonido y vídeo (T15, T23). Se escribe de forma atómica; si falta o está dañada, valores por
//! defecto, y los campos que falten toman el suyo.
//!
//! Basado en la configuración de Digityzer (`crates/digityzer-app/src/ui/config.rs`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::player::AudioServer;
use crate::theme::Theme;

/// Nº máximo de bases recientes.
pub const MAX_RECENT: usize = 8;

/// Configuración guardada.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Identificador del tema (`dracula`, `nord`…).
    pub theme: String,
    /// Bases abiertas recientemente, la última primero (se reabre al arrancar).
    pub recent_bases: Vec<PathBuf>,
    /// Carpeta donde se crean las bases nuevas (vacía = `~/.local/share/atenea/bases`).
    pub bases_dir: PathBuf,
    /// Servidor de sonido.
    pub audio: AudioServer,
    /// Dispositivo de audio (vacío = el de por defecto).
    pub audio_device: String,
    /// Mostrar vídeo al reproducir.
    pub video: bool,
    /// Volumen 0–100.
    pub volume: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default_theme().id.into(),
            recent_bases: Vec::new(),
            bases_dir: PathBuf::new(),
            audio: AudioServer::default(),
            audio_device: String::new(),
            video: false,
            volume: 80.0,
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
    /// Pone `path` la primera de las recientes (sin repetir, como mucho [`MAX_RECENT`]).
    pub fn push_recent(&mut self, path: &Path) {
        self.recent_bases.retain(|p| p != path);
        self.recent_bases.insert(0, path.to_owned());
        self.recent_bases.truncate(MAX_RECENT);
    }

    /// Quita una base de las recientes (p. ej. si ya no existe).
    pub fn forget_recent(&mut self, path: &Path) {
        self.recent_bases.retain(|p| p != path);
    }

    /// Carpeta para las bases nuevas.
    pub fn bases_dir(&self) -> Option<PathBuf> {
        if self.bases_dir.as_os_str().is_empty() {
            crate::userdb::data_dir().map(|d| d.join("bases"))
        } else {
            Some(self.bases_dir.clone())
        }
    }

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
        let mut c = Config {
            theme: "nord".into(),
            audio: AudioServer::Alsa,
            audio_device: "hw:1,0".into(),
            ..Config::default()
        };
        c.push_recent(Path::new("/b/uno.db"));
        c.push_recent(Path::new("/b/dos.db"));
        c.push_recent(Path::new("/b/uno.db"));
        assert_eq!(
            c.recent_bases,
            [PathBuf::from("/b/uno.db"), "/b/dos.db".into()]
        );
        for i in 0..20 {
            c.push_recent(Path::new(&format!("/b/{i}.db")));
        }
        assert_eq!(c.recent_bases.len(), MAX_RECENT);
        c.forget_recent(Path::new("/b/19.db"));
        assert_eq!(c.recent_bases[0], PathBuf::from("/b/18.db"));
        c.save_to(&p).unwrap();
        assert_eq!(Config::load_from(&p), c);
        std::fs::write(&p, "theme = 3\n").unwrap();
        assert_eq!(Config::load_from(&p), Config::default());
        std::fs::write(&p, "theme = \"gruvbox-dark\"\n").unwrap();
        let old = Config::load_from(&p);
        assert_eq!(old.theme, "gruvbox-dark");
        assert_eq!(
            old.volume, 80.0,
            "los campos nuevos toman su valor por defecto"
        );
    }
}
