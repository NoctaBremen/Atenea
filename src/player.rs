//! Reproducción con mpv + yt-dlp (T15), controlada por el socket IPC JSON de mpv.
//!
//! Se lanza un único proceso `mpv --idle` que vive mientras la app esté abierta. Por defecto
//! solo audio; el vídeo se enciende y apaga en caliente con la propiedad `vid`, en una ventana
//! pequeña con `app-id`/clase `vault-video` para poder hacerla flotante en Hyprland o mango:
//!
//! ```text
//! windowrulev2 = float, class:^(vault-video)$
//! ```
//!
//! Los mensajes de mpv (respuestas y cambios de propiedades) llegan por un canal como
//! [`PlayerEvent`].

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Servidor de sonido.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AudioServer {
    /// PipeWire (por defecto, como cualquier escritorio actual).
    #[default]
    PipeWire,
    /// PulseAudio (o pipewire-pulse).
    PulseAudio,
    /// ALSA directo, con un dispositivo concreto (`hw:1,0`) o el de por defecto.
    Alsa,
}

impl AudioServer {
    /// Todos, en el orden de los ajustes.
    pub const ALL: [AudioServer; 3] = [
        AudioServer::PipeWire,
        AudioServer::PulseAudio,
        AudioServer::Alsa,
    ];

    /// Nombre de la salida de audio de mpv (`--ao`).
    pub fn mpv_ao(self) -> &'static str {
        match self {
            AudioServer::PipeWire => "pipewire",
            AudioServer::PulseAudio => "pulse",
            AudioServer::Alsa => "alsa",
        }
    }

    /// Nombre visible.
    pub fn label(self) -> &'static str {
        match self {
            AudioServer::PipeWire => "PipeWire",
            AudioServer::PulseAudio => "PulseAudio",
            AudioServer::Alsa => "ALSA",
        }
    }
}

/// Cómo se lanza mpv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerOptions {
    /// Servidor de sonido.
    pub audio: AudioServer,
    /// Dispositivo (vacío = el de por defecto). Para ALSA, p. ej. `hw:1,0`.
    pub device: String,
    /// Empezar con vídeo.
    pub video: bool,
    /// Salida de audio que sustituye a la del servidor (`null` en tests).
    pub ao_override: Option<String>,
}

impl PlayerOptions {
    /// Argumentos de mpv (sin el socket).
    pub fn args(&self) -> Vec<String> {
        let ao = self
            .ao_override
            .clone()
            .unwrap_or_else(|| self.audio.mpv_ao().to_owned());
        let mut a = vec![
            "--idle=yes".to_owned(),
            "--no-terminal".to_owned(),
            "--no-config".to_owned(),
            format!("--ao={ao}"),
            "--force-window=no".to_owned(),
            "--keep-open=no".to_owned(),
            "--geometry=480x270-24-24".to_owned(),
            "--autofit-larger=640x360".to_owned(),
            "--title=Vault — ${media-title}".to_owned(),
            "--x11-name=vault-video".to_owned(),
            "--wayland-app-id=vault-video".to_owned(),
            "--ytdl=yes".to_owned(),
            // Solo audio no necesita bajar vídeo: formato de audio si lo hay.
            "--ytdl-format=bestaudio/best".to_owned(),
            format!("--vid={}", if self.video { "auto" } else { "no" }),
        ];
        if !self.device.trim().is_empty() {
            a.push(format!("--audio-device={}/{}", ao, self.device.trim()));
        }
        a
    }
}

/// Algo que ha pasado en mpv.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// Posición en segundos.
    Position(f64),
    /// Duración en segundos.
    Duration(f64),
    /// Pausado o no.
    Paused(bool),
    /// Título de lo que suena.
    Title(String),
    /// Volumen (0–100).
    Volume(f64),
    /// Empieza a cargar un archivo.
    Loading,
    /// Ha terminado lo que sonaba (`reason`: `eof`, `stop`, `error`…).
    Ended(String),
    /// No hay nada cargado.
    Idle,
    /// mpv se ha cerrado.
    Exited,
}

/// Mensaje recibido por el socket, interpretado.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Respuesta a un comando.
    Reply {
        /// Id del comando.
        request_id: u64,
        /// `success` u otro texto de error.
        error: String,
        /// Datos devueltos.
        data: Value,
    },
    /// Evento.
    Event(PlayerEvent),
    /// Otra cosa que no interesa.
    Other,
}

/// Ids de las propiedades observadas.
const OBSERVED: [(u64, &str); 5] = [
    (1, "time-pos"),
    (2, "duration"),
    (3, "pause"),
    (4, "media-title"),
    (5, "volume"),
];

/// Interpreta una línea JSON del socket de mpv.
pub fn parse_line(line: &str) -> Incoming {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Incoming::Other;
    };
    if let Some(id) = v.get("request_id").and_then(Value::as_u64) {
        return Incoming::Reply {
            request_id: id,
            error: v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            data: v.get("data").cloned().unwrap_or(Value::Null),
        };
    }
    let Some(ev) = v.get("event").and_then(Value::as_str) else {
        return Incoming::Other;
    };
    let data = v.get("data");
    let num = || data.and_then(Value::as_f64);
    Incoming::Event(match ev {
        "property-change" => match v.get("name").and_then(Value::as_str) {
            Some("time-pos") => match num() {
                Some(x) => PlayerEvent::Position(x),
                None => return Incoming::Other,
            },
            Some("duration") => match num() {
                Some(x) => PlayerEvent::Duration(x),
                None => return Incoming::Other,
            },
            Some("pause") => PlayerEvent::Paused(data.and_then(Value::as_bool).unwrap_or(false)),
            Some("media-title") => {
                PlayerEvent::Title(data.and_then(Value::as_str).unwrap_or("").to_owned())
            }
            Some("volume") => match num() {
                Some(x) => PlayerEvent::Volume(x),
                None => return Incoming::Other,
            },
            _ => return Incoming::Other,
        },
        "start-file" => PlayerEvent::Loading,
        "end-file" => PlayerEvent::Ended(
            v.get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ),
        "idle" => PlayerEvent::Idle,
        "shutdown" => PlayerEvent::Exited,
        _ => return Incoming::Other,
    })
}

/// Línea JSON de un comando con su id.
pub fn command_line(request_id: u64, args: &[Value]) -> String {
    let mut s = json!({ "command": args, "request_id": request_id }).to_string();
    s.push('\n');
    s
}

/// Error del reproductor.
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    /// Falta un programa.
    #[error("no se encuentra «{0}»: instálalo para poder escuchar (p. ej. pacman -S {0})")]
    Missing(&'static str),
    /// E/S (lanzar mpv, socket).
    #[error("error con mpv: {0}")]
    Io(#[from] std::io::Error),
    /// mpv no abrió el socket a tiempo.
    #[error("mpv no ha respondido")]
    Timeout,
    /// mpv rechazó un comando.
    #[error("mpv: {0}")]
    Command(String),
}

/// ¿Está este programa en el `PATH`?
pub fn in_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let p = dir.join(program);
            p.is_file()
                && std::fs::metadata(&p)
                    .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0)
                    .unwrap_or(false)
        })
    })
}

/// Comprueba que existen `mpv` y `yt-dlp`.
pub fn check_programs() -> Result<(), PlayerError> {
    if !in_path("mpv") {
        return Err(PlayerError::Missing("mpv"));
    }
    if !in_path("yt-dlp") {
        return Err(PlayerError::Missing("yt-dlp"));
    }
    Ok(())
}

type Pending = Arc<Mutex<Vec<(u64, Sender<Result<Value, String>>)>>>;

/// Conexión con un mpv (lanzado por nosotros o ya existente en un socket).
pub struct Player {
    child: Option<Child>,
    socket_path: PathBuf,
    writer: Mutex<UnixStream>,
    next_id: AtomicU64,
    pending: Pending,
    events: Mutex<Option<Receiver<PlayerEvent>>>,
}

impl Player {
    /// Lanza mpv y se conecta a su socket.
    pub fn launch(opts: &PlayerOptions) -> Result<Self, PlayerError> {
        check_programs()?;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let socket_path = dir.join(format!("vault-mpv-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        let child = Command::new("mpv")
            .args(opts.args())
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match UnixStream::connect(&socket_path) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(_) => {
                    let mut child = child;
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(PlayerError::Timeout);
                }
            }
        };
        let mut p = Self::from_stream(stream, socket_path)?;
        p.child = Some(child);
        Ok(p)
    }

    /// Sobre un socket ya conectado (tests).
    pub fn from_stream(stream: UnixStream, socket_path: PathBuf) -> Result<Self, PlayerError> {
        let reader = stream.try_clone()?;
        let pending: Pending = Arc::default();
        let (tx, rx) = channel();
        {
            let pending = Arc::clone(&pending);
            std::thread::Builder::new()
                .name("mpv-ipc".into())
                .spawn(move || read_loop(reader, &pending, &tx))?;
        }
        let p = Self {
            child: None,
            socket_path,
            writer: Mutex::new(stream),
            next_id: AtomicU64::new(100),
            pending,
            events: Mutex::new(Some(rx)),
        };
        for (id, name) in OBSERVED {
            p.send(&[json!("observe_property"), json!(id), json!(name)])?;
        }
        Ok(p)
    }

    /// Canal de eventos. Solo se puede tomar una vez.
    pub fn take_events(&self) -> Option<Receiver<PlayerEvent>> {
        self.events.lock().ok()?.take()
    }

    /// Ruta del socket.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Envía un comando sin esperar respuesta.
    pub fn send(&self, args: &[Value]) -> Result<u64, PlayerError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = command_line(id, args);
        let mut w = self
            .writer
            .lock()
            .map_err(|_| PlayerError::Command("socket bloqueado".into()))?;
        w.write_all(line.as_bytes())?;
        Ok(id)
    }

    /// Envía un comando y espera su respuesta (hasta 2 s).
    pub fn request(&self, args: &[Value]) -> Result<Value, PlayerError> {
        let (tx, rx) = channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut p) = self.pending.lock() {
            p.push((id, tx));
        }
        {
            let mut w = self
                .writer
                .lock()
                .map_err(|_| PlayerError::Command("socket bloqueado".into()))?;
            w.write_all(command_line(id, args).as_bytes())?;
        }
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(PlayerError::Command(e)),
            Err(_) => Err(PlayerError::Timeout),
        }
    }

    /// Reproduce una URL (sustituye lo que suene).
    pub fn play(&self, url: &str) -> Result<(), PlayerError> {
        self.send(&[json!("loadfile"), json!(url), json!("replace")])?;
        self.set_pause(false)
    }

    /// Pausa o reanuda.
    pub fn set_pause(&self, pause: bool) -> Result<(), PlayerError> {
        self.send(&[json!("set_property"), json!("pause"), json!(pause)])
            .map(|_| ())
    }

    /// Alterna pausa.
    pub fn toggle_pause(&self) -> Result<(), PlayerError> {
        self.send(&[json!("cycle"), json!("pause")]).map(|_| ())
    }

    /// Para y descarga lo que suena.
    pub fn stop(&self) -> Result<(), PlayerError> {
        self.send(&[json!("stop")]).map(|_| ())
    }

    /// Salta `secs` segundos (negativo hacia atrás).
    pub fn seek_relative(&self, secs: f64) -> Result<(), PlayerError> {
        self.send(&[json!("seek"), json!(secs), json!("relative")])
            .map(|_| ())
    }

    /// Va a la posición `secs`.
    pub fn seek_absolute(&self, secs: f64) -> Result<(), PlayerError> {
        self.send(&[json!("seek"), json!(secs.max(0.0)), json!("absolute")])
            .map(|_| ())
    }

    /// Volumen 0–100.
    pub fn set_volume(&self, v: f64) -> Result<(), PlayerError> {
        self.send(&[
            json!("set_property"),
            json!("volume"),
            json!(v.clamp(0.0, 100.0)),
        ])
        .map(|_| ())
    }

    /// Muestra u oculta la ventana de vídeo.
    pub fn set_video(&self, on: bool) -> Result<(), PlayerError> {
        self.send(&[
            json!("set_property"),
            json!("vid"),
            json!(if on { "auto" } else { "no" }),
        ])
        .map(|_| ())
    }

    /// Lee una propiedad.
    pub fn get(&self, name: &str) -> Result<Value, PlayerError> {
        self.request(&[json!("get_property"), json!(name)])
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = self.send(&[json!("quit")]);
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

fn read_loop(stream: UnixStream, pending: &Pending, events: &Sender<PlayerEvent>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        match parse_line(&line) {
            Incoming::Reply {
                request_id,
                error,
                data,
            } => {
                let waiter = pending.lock().ok().and_then(|mut p| {
                    p.iter()
                        .position(|(id, _)| *id == request_id)
                        .map(|i| p.swap_remove(i).1)
                });
                if let Some(tx) = waiter {
                    let _ = tx.send(if error == "success" {
                        Ok(data)
                    } else {
                        Err(error)
                    });
                }
            }
            Incoming::Event(e) => {
                let _ = events.send(e);
            }
            Incoming::Other => {}
        }
    }
    let _ = events.send(PlayerEvent::Exited);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argumentos() {
        let o = PlayerOptions::default();
        let a = o.args();
        assert!(a.contains(&"--ao=pipewire".to_owned()));
        assert!(a.contains(&"--vid=no".to_owned()));
        assert!(!a.iter().any(|x| x.starts_with("--audio-device")));
        let o = PlayerOptions {
            audio: AudioServer::Alsa,
            device: "hw:1,0".into(),
            video: true,
            ao_override: None,
        };
        let a = o.args();
        assert!(a.contains(&"--ao=alsa".to_owned()));
        assert!(a.contains(&"--audio-device=alsa/hw:1,0".to_owned()));
        assert!(a.contains(&"--vid=auto".to_owned()));
    }

    #[test]
    fn mensajes() {
        assert_eq!(
            parse_line(r#"{"data":12.5,"error":"success","request_id":7}"#),
            Incoming::Reply {
                request_id: 7,
                error: "success".into(),
                data: json!(12.5)
            }
        );
        assert_eq!(
            parse_line(r#"{"event":"property-change","id":1,"name":"time-pos","data":3.25}"#),
            Incoming::Event(PlayerEvent::Position(3.25))
        );
        assert_eq!(
            parse_line(r#"{"event":"property-change","id":1,"name":"time-pos"}"#),
            Incoming::Other
        );
        assert_eq!(
            parse_line(r#"{"event":"property-change","name":"pause","data":true}"#),
            Incoming::Event(PlayerEvent::Paused(true))
        );
        assert_eq!(
            parse_line(r#"{"event":"end-file","reason":"eof","playlist_entry_id":1}"#),
            Incoming::Event(PlayerEvent::Ended("eof".into()))
        );
        assert_eq!(parse_line("no es json"), Incoming::Other);
        assert_eq!(
            command_line(3, &[json!("seek"), json!(-10), json!("relative")]),
            "{\"command\":[\"seek\",-10,\"relative\"],\"request_id\":3}\n"
        );
    }

    #[test]
    fn ida_y_vuelta_con_un_mpv_falso() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        // «mpv» falso: responde a cada comando y emite un evento de posición.
        std::thread::spawn(move || {
            let mut w = theirs.try_clone().unwrap();
            for line in BufReader::new(theirs).lines() {
                let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
                let id = v["request_id"].as_u64().unwrap();
                let reply = if v["command"][0] == "get_property" {
                    json!({"data": "Kalo Tren - Axis Point", "error": "success", "request_id": id})
                } else {
                    json!({"data": null, "error": "success", "request_id": id})
                };
                writeln!(w, "{reply}").unwrap();
                if v["command"][0] == "seek" {
                    writeln!(
                        w,
                        r#"{{"event":"property-change","name":"time-pos","data":10.0}}"#
                    )
                    .unwrap();
                }
            }
        });
        let p = Player::from_stream(ours, PathBuf::from("/nada")).unwrap();
        let ev = p.take_events().unwrap();
        assert!(p.take_events().is_none());
        assert_eq!(
            p.get("media-title").unwrap(),
            json!("Kalo Tren - Axis Point")
        );
        p.seek_relative(10.0).unwrap();
        let got = ev.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(got, PlayerEvent::Position(10.0));
    }
}
