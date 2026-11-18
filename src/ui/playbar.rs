//! Barra de reproducción (T16): vídeo anterior/siguiente del disco, ±10 s, play/pausa, barra
//! de posición, título, volumen, vídeo sí/no y modo digging (T17).

use std::cell::Cell;

use gtk::prelude::*;

use super::detail::format_secs;

/// La barra. La ventana conecta las señales de cada botón.
pub struct PlayBar {
    /// Widget raíz.
    pub root: gtk::Box,
    /// Vídeo anterior.
    pub prev: gtk::Button,
    /// −10 s.
    pub back: gtk::Button,
    /// Play/pausa.
    pub play: gtk::Button,
    /// +10 s.
    pub fwd: gtk::Button,
    /// Vídeo siguiente.
    pub next: gtk::Button,
    /// Posición.
    pub scale: gtk::Scale,
    /// Título.
    pub title: gtk::Label,
    /// Tiempo.
    pub time: gtk::Label,
    /// Mostrar vídeo.
    pub video: gtk::ToggleButton,
    /// Volumen.
    pub volume: gtk::Scale,
    /// Modo digging.
    pub digging: gtk::ToggleButton,
    duration: Cell<f64>,
    /// Mientras el usuario arrastra la barra no se mueve sola.
    pub dragging: Cell<bool>,
    /// Cambios de la barra hechos por código (no son búsquedas del usuario).
    pub quiet: Cell<bool>,
}

fn icon(name: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(name);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    b
}

impl PlayBar {
    /// Construye la barra.
    pub fn new(volume: f64, video: bool) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.add_css_class("playbar");
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.set_margin_top(4);
        root.set_margin_bottom(4);

        let prev = icon(
            "media-skip-backward-symbolic",
            "Vídeo anterior del disco (,)",
        );
        let back = icon("media-seek-backward-symbolic", "−10 s (←)");
        let play = icon("media-playback-start-symbolic", "Play/pausa (Espacio)");
        let fwd = icon("media-seek-forward-symbolic", "+10 s (→)");
        let next = icon(
            "media-skip-forward-symbolic",
            "Vídeo siguiente del disco (.)",
        );
        for b in [&prev, &back, &play, &fwd, &next] {
            root.append(b);
        }

        let middle = gtk::Box::new(gtk::Orientation::Vertical, 0);
        middle.set_hexpand(true);
        let title = gtk::Label::builder()
            .label("Nada sonando")
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        scale.set_hexpand(true);
        scale.set_draw_value(false);
        scale.set_sensitive(false);
        let time = gtk::Label::new(Some("0:00 / 0:00"));
        time.add_css_class("dim-label");
        time.add_css_class("numeric");
        line.append(&scale);
        line.append(&time);
        middle.append(&title);
        middle.append(&line);
        root.append(&middle);

        let video_t = gtk::ToggleButton::builder()
            .icon_name("video-display-symbolic")
            .tooltip_text("Mostrar vídeo (V)")
            .active(video)
            .build();
        video_t.add_css_class("flat");
        root.append(&video_t);

        let vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        vol.set_value(volume);
        vol.set_draw_value(false);
        vol.set_width_request(100);
        vol.set_tooltip_text(Some("Volumen"));
        root.append(&gtk::Image::from_icon_name("audio-volume-medium-symbolic"));
        root.append(&vol);

        let digging = gtk::ToggleButton::with_label("Digging");
        digging.set_tooltip_text(Some(
            "Modo digging (D): recorre los resultados en cola.\n\
             1–4 marcan y saltan al siguiente disco · N siguiente · P anterior",
        ));
        root.append(&digging);

        Self {
            root,
            prev,
            back,
            play,
            fwd,
            next,
            scale,
            title,
            time,
            video: video_t,
            volume: vol,
            digging,
            duration: Cell::new(0.0),
            dragging: Cell::new(false),
            quiet: Cell::new(false),
        }
    }

    /// Título de lo que suena.
    pub fn set_title(&self, t: &str) {
        self.title
            .set_label(if t.is_empty() { "Nada sonando" } else { t });
    }

    /// Icono de play o pausa.
    pub fn set_paused(&self, paused: bool) {
        self.play.set_icon_name(if paused {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        });
    }

    /// Duración del vídeo.
    pub fn set_duration(&self, d: f64) {
        self.duration.set(d);
        self.quiet.set(true);
        self.scale.set_range(0.0, d.max(1.0));
        self.quiet.set(false);
        self.scale.set_sensitive(d > 0.0);
    }

    /// Posición.
    pub fn set_position(&self, p: f64) {
        if !self.dragging.get() {
            self.quiet.set(true);
            self.scale.set_value(p);
            self.quiet.set(false);
        }
        self.time.set_label(&format!(
            "{} / {}",
            format_secs(p),
            format_secs(self.duration.get())
        ));
    }

    /// Vuelve al estado de reposo.
    pub fn reset(&self) {
        self.set_title("");
        self.set_paused(true);
        self.set_duration(0.0);
        self.set_position(0.0);
    }
}
