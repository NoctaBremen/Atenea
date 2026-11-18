//! Aprovechar la RAM: la base se mapea en memoria (`mmap_size`) y, si cabe holgadamente, se
//! precarga leyéndola una vez en segundo plano. Así las búsquedas leen de RAM desde el
//! principio en vez de ir al disco página a página.
//!
//! La memoria la gestiona el sistema operativo (caché de páginas): si otro programa la
//! necesita, la recupera sin más. Vault no reserva nada.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cómo usar la memoria con una base concreta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPlan {
    /// Bytes de la base que SQLite mapea en memoria.
    pub mmap_bytes: u64,
    /// Precargar la base al abrirla.
    pub warm: bool,
}

/// Memoria disponible según `/proc/meminfo` (`MemAvailable`), en bytes.
pub fn available_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&text)
}

/// Lee `MemAvailable` de un texto con el formato de `/proc/meminfo`.
pub fn parse_meminfo(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix("MemAvailable:")?;
        let kib: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kib * 1024)
    })
}

/// Decide cuánto mapear. Si la base ocupa como mucho la mitad de la memoria libre, entera y
/// precargada; si no, una cuarta parte de la memoria libre y sin precarga (para no echar a
/// otros programas). Sin datos de memoria, 2 GiB sin precarga.
pub fn plan(file_size: u64, available: Option<u64>) -> MemoryPlan {
    const FALLBACK: u64 = 2 << 30;
    match available {
        Some(av) if file_size <= av / 2 => MemoryPlan {
            // Un poco de margen: la base de usuario y lo que crezca.
            mmap_bytes: file_size + (64 << 20),
            warm: true,
        },
        Some(av) => MemoryPlan {
            mmap_bytes: av / 4,
            warm: false,
        },
        None => MemoryPlan {
            mmap_bytes: FALLBACK,
            warm: false,
        },
    }
}

/// Plan para la base de `path`.
pub fn plan_for(path: &Path) -> MemoryPlan {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    plan(size, available_bytes())
}

/// Lee el archivo entero para dejarlo en la caché de páginas del sistema. Para si `cancel` se
/// activa (p. ej. al cambiar de base). Devuelve los bytes leídos.
pub fn warm_file(path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 4 << 20];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_y_plan() {
        let text = "MemTotal:       65536000 kB\nMemFree:  1000 kB\nMemAvailable:   60000000 kB\n";
        assert_eq!(parse_meminfo(text), Some(60_000_000 * 1024));
        assert_eq!(parse_meminfo("MemTotal: 1 kB"), None);

        let gib = 1u64 << 30;
        // 1,7 GB con ~57 GB libres: entera y precargada.
        let p = plan(1_700_000_000, Some(57 * gib));
        assert!(p.warm);
        assert!(p.mmap_bytes >= 1_700_000_000);
        // 26 GB con 40 GB libres: no cabe holgadamente.
        let p = plan(26 * gib, Some(40 * gib));
        assert!(!p.warm);
        assert_eq!(p.mmap_bytes, 10 * gib);
        assert_eq!(plan(5, None).mmap_bytes, 2 * gib);
        assert!(
            available_bytes().is_some(),
            "en Linux siempre hay /proc/meminfo"
        );
    }

    #[test]
    fn precarga_lee_todo_y_se_puede_cancelar() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.db");
        std::fs::write(&p, vec![7u8; 10 << 20]).unwrap();
        assert_eq!(warm_file(&p, &AtomicBool::new(false)).unwrap(), 10 << 20);
        assert_eq!(warm_file(&p, &AtomicBool::new(true)).unwrap(), 0);
    }
}
