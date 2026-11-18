//! Búsquedas en un hilo aparte (T11): la interfaz nunca espera a SQLite.
//!
//! El hilo tiene su propia conexión. Cada petición lleva un número de generación; si llegan
//! varias seguidas solo se atiende la última, y la que esté en curso se interrumpe.

use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use atenea::db::{Base, Filtros, Sort};

/// Una búsqueda.
#[derive(Debug, Clone)]
pub struct Request {
    /// Generación (la interfaz descarta respuestas viejas).
    pub generation: u64,
    /// Base de Discogs.
    pub base: PathBuf,
    /// Base de usuario.
    pub user: Option<PathBuf>,
    /// Filtros.
    pub filtros: Filtros,
    /// Orden.
    pub sort: Sort,
}

/// Respuesta.
#[derive(Debug)]
pub struct Response {
    /// Generación de la petición.
    pub generation: u64,
    /// Ids o mensaje de error.
    pub result: Result<Vec<i64>, String>,
    /// Tiempo empleado.
    pub elapsed: Duration,
}

/// Hilo de búsqueda.
pub struct SearchWorker {
    tx: Sender<Request>,
    interrupt: Arc<Mutex<Option<rusqlite::InterruptHandle>>>,
}

impl SearchWorker {
    /// Arranca el hilo. Las respuestas llegan por `out`.
    pub fn spawn(out: async_channel::Sender<Response>) -> std::io::Result<Self> {
        let (tx, rx) = channel::<Request>();
        let interrupt: Arc<Mutex<Option<rusqlite::InterruptHandle>>> = Arc::default();
        let handle_slot = Arc::clone(&interrupt);
        std::thread::Builder::new()
            .name("atenea-busqueda".into())
            .spawn(move || {
                let mut open: Option<(PathBuf, Base)> = None;
                while let Ok(mut req) = rx.recv() {
                    // Solo la última de las pendientes.
                    while let Ok(newer) = rx.try_recv() {
                        req = newer;
                    }
                    let start = Instant::now();
                    if open.as_ref().is_none_or(|(p, _)| *p != req.base) {
                        open = match Base::open(&req.base, req.user.as_deref()) {
                            Ok(b) => {
                                if let Ok(mut slot) = handle_slot.lock() {
                                    *slot = Some(b.interrupt_handle());
                                }
                                Some((req.base.clone(), b))
                            }
                            Err(e) => {
                                let _ = out.send_blocking(Response {
                                    generation: req.generation,
                                    result: Err(e.to_string()),
                                    elapsed: start.elapsed(),
                                });
                                None
                            }
                        };
                    }
                    let Some((_, base)) = open.as_ref() else {
                        continue;
                    };
                    let result = base
                        .ids(&req.filtros, req.sort, None)
                        .map_err(|e| e.to_string());
                    // Una consulta interrumpida no se contesta: ya hay otra detrás.
                    if matches!(&result, Err(e) if e.contains("interrupted")) {
                        continue;
                    }
                    if out
                        .send_blocking(Response {
                            generation: req.generation,
                            result,
                            elapsed: start.elapsed(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { tx, interrupt })
    }

    /// Pide una búsqueda e interrumpe la que esté en curso.
    pub fn request(&self, req: Request) {
        if let Ok(slot) = self.interrupt.lock()
            && let Some(h) = slot.as_ref()
        {
            h.interrupt();
        }
        let _ = self.tx.send(req);
    }
}
