#![allow(clippy::unwrap_used, clippy::expect_used)]
//! T15 con mpv de verdad (se salta si no están mpv y ffmpeg): un tono generado al vuelo,
//! salida de audio `null`.

use std::time::{Duration, Instant};

use atenea::player::{Player, PlayerEvent, PlayerOptions, in_path};

fn wait_for(
    ev: &std::sync::mpsc::Receiver<PlayerEvent>,
    secs: u64,
    pred: impl Fn(&PlayerEvent) -> bool,
) -> Option<PlayerEvent> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match ev.recv_timeout(left) {
            Ok(e) if pred(&e) => return Some(e),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

#[test]
fn reproduce_pausa_y_salta() {
    if !(in_path("mpv") && in_path("yt-dlp") && in_path("ffmpeg")) {
        eprintln!("sin mpv/yt-dlp/ffmpeg: se salta");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("tono.wav");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=5",
        ])
        .arg(&wav)
        .status()
        .unwrap()
        .success();
    assert!(ok);

    let p = Player::launch(&PlayerOptions {
        ao_override: Some("null".into()),
        ..Default::default()
    })
    .unwrap();
    let ev = p.take_events().unwrap();
    p.play(wav.to_str().unwrap()).unwrap();
    let d = wait_for(&ev, 10, |e| matches!(e, PlayerEvent::Duration(_))).unwrap();
    let PlayerEvent::Duration(d) = d else {
        unreachable!()
    };
    assert!((d - 5.0).abs() < 0.2, "{d}");
    wait_for(
        &ev,
        5,
        |e| matches!(e, PlayerEvent::Position(x) if *x > 0.3),
    )
    .unwrap();

    p.set_pause(true).unwrap();
    wait_for(&ev, 5, |e| *e == PlayerEvent::Paused(true)).unwrap();
    p.seek_absolute(4.0).unwrap();
    wait_for(
        &ev,
        5,
        |e| matches!(e, PlayerEvent::Position(x) if *x >= 3.9),
    )
    .unwrap();
    assert_eq!(p.get("pause").unwrap(), serde_json::json!(true));
    p.set_volume(40.0).unwrap();
    wait_for(&ev, 5, |e| *e == PlayerEvent::Volume(40.0)).unwrap();
    p.set_pause(false).unwrap();
    wait_for(&ev, 10, |e| *e == PlayerEvent::Ended("eof".into())).unwrap();

    let sock = p.socket_path().to_owned();
    drop(p);
    assert!(!sock.exists(), "el socket se borra al cerrar");
}
