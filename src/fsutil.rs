//! Utilidades de archivos: escritura atómica.
//!
//! Copiado de Digityzer (`crates/audio-core/src/fsutil.rs`, commit 1f7e585).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Prefijo de los temporales.
pub const TEMP_PREFIX: &str = ".atenea-tmp-";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Ruta temporal en el mismo directorio que `dest`: `.<nombre>.atenea-tmp-<pid>-<n>`.
pub fn temp_path_for(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = format!(".{name}{TEMP_PREFIX}{}-{n}", std::process::id());
    dest.with_file_name(tmp)
}

/// Escribe `bytes` en `dest` de forma atómica: temporal en el mismo directorio → `fsync` →
/// rename (sustituye el archivo si existe) → `fsync` del directorio. Crea el directorio si no
/// existe. El temporal se borra si algo falla.
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = temp_path_for(dest);
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, dest)?;
        // El fsync del directorio hace duradero el rename; si el sistema de archivos no lo
        // admite, no es un error.
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escribe_y_sustituye_sin_dejar_temporales() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("a.txt");
        write_atomic(&p, b"uno").unwrap();
        write_atomic(&p, b"dos").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"dos");
        let n = fs::read_dir(p.parent().unwrap()).unwrap().count();
        assert_eq!(n, 1);
    }
}
