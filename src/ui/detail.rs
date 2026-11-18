//! Panel de detalle (T13): datos del disco, tracklist con duraciones, créditos, notas de
//! Discogs, vídeos (▶ para escuchar), botón «Abrir en Discogs», marcador, maletas (T20) y tu
//! nota (T22). Los dumps no traen portadas.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use atenea::db::{Detail, Mark};
use atenea::userdb::Suitcase;
use gtk::glib;
use gtk::prelude::*;

/// Lo que el panel pide a la ventana.
pub struct DetailActions {
    /// Escuchar el vídeo `i` del disco mostrado.
    pub play_video: Box<dyn Fn(usize)>,
    /// Poner o quitar marcador.
    pub set_mark: Box<dyn Fn(Option<Mark>)>,
    /// Meter (`true`) o sacar el disco de una maleta.
    pub set_suitcase: Box<dyn Fn(i64, bool)>,
    /// Guardar la nota.
    pub save_note: Box<dyn Fn(i64, String)>,
}

/// El panel.
pub struct DetailPanel {
    /// Widget raíz.
    pub root: gtk::ScrolledWindow,
    content: gtk::Box,
    actions: Rc<DetailActions>,
    current: Cell<Option<i64>>,
    note: gtk::TextView,
    note_timer: RefCell<Option<glib::SourceId>>,
    quiet: Cell<bool>,
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .css_classes(classes.to_vec())
        .build()
}

fn heading(text: &str) -> gtk::Label {
    let l = label(text, &["dim-label", "section-title"]);
    l.set_selectable(false);
    l.set_margin_top(10);
    l
}

/// Segundos → «m:ss» o «h:mm:ss».
pub fn format_secs(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl DetailPanel {
    /// Panel vacío.
    pub fn new(actions: DetailActions) -> Rc<Self> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(10);
        content.set_margin_bottom(10);
        // Ancho fijo: con barra horizontal automática el panel no crece con lo que muestra
        // (la lista no se mueve al cambiar de disco).
        let root = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_width(false)
            .build();
        let note = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .height_request(70)
            .css_classes(["note"])
            .build();
        let this = Rc::new(Self {
            root,
            content,
            actions: Rc::new(actions),
            current: Cell::new(None),
            note,
            note_timer: RefCell::default(),
            quiet: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.note.buffer().connect_changed(move |buf| {
            let Some(this) = weak.upgrade() else { return };
            if this.quiet.get() {
                return;
            }
            let Some(id) = this.current.get() else { return };
            if let Some(t) = this.note_timer.take() {
                t.remove();
            }
            let text = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            let weak = Rc::downgrade(&this);
            let timer =
                glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                    if let Some(this) = weak.upgrade() {
                        this.note_timer.replace(None);
                        (this.actions.save_note)(id, text);
                    }
                });
            this.note_timer.replace(Some(timer));
        });
        this.show_empty("Elige un disco de la lista.");
        this
    }

    fn clear(&self) {
        // Si había una nota sin guardar, se guarda ya.
        if let Some(t) = self.note_timer.take() {
            t.remove();
            if let Some(id) = self.current.get() {
                let b = self.note.buffer();
                (self.actions.save_note)(
                    id,
                    b.text(&b.start_iter(), &b.end_iter(), false).to_string(),
                );
            }
        }
        while let Some(c) = self.content.first_child() {
            self.content.remove(&c);
        }
    }

    /// Mensaje en lugar de un disco.
    pub fn show_empty(&self, msg: &str) {
        self.clear();
        self.current.set(None);
        let l = label(msg, &["dim-label"]);
        l.set_margin_top(20);
        self.content.append(&l);
    }

    /// Disco mostrado.
    pub fn current(&self) -> Option<i64> {
        self.current.get()
    }

    /// Muestra un disco.
    pub fn show(
        &self,
        d: &Detail,
        mark: Option<Mark>,
        note: &str,
        suitcases: &[Suitcase],
        in_suitcases: &[i64],
        playing_video: Option<usize>,
    ) {
        self.clear();
        self.current.set(Some(d.row.id));
        let r = &d.row;
        let c = &self.content;
        c.append(&label(&r.artist, &["detail-artist"]));
        c.append(&label(&r.title, &["detail-title"]));

        let mut facts = Vec::new();
        for (l, catno) in &d.labels {
            facts.push(if catno.is_empty() {
                l.clone()
            } else {
                format!("{l} — {catno}")
            });
        }
        let date = if d.released.is_empty() {
            r.year.map(|y| y.to_string()).unwrap_or_default()
        } else {
            d.released.trim_end_matches("-00").to_owned()
        };
        let place = [r.country.as_str(), date.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        if !place.is_empty() {
            facts.push(place);
        }
        facts.push(r.format.clone());
        let gs = [d.genres.as_str(), r.styles.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        if !gs.is_empty() {
            facts.push(gs);
        }
        for f in facts.iter().filter(|f| !f.is_empty()) {
            c.append(&label(f, &["detail-fact"]));
        }

        // Marcador. Sin grupo de radio para poder quitarlo pulsando el activo; la ventana vuelve
        // a pintar el panel tras cada cambio.
        c.append(&heading("Marcador"));
        let marks = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(2)
            .row_spacing(4)
            .column_spacing(4)
            .build();
        for (i, m) in Mark::ALL.iter().enumerate() {
            let t = gtk::ToggleButton::with_label(m.label());
            t.set_tooltip_text(Some(&format!("Tecla {}", i + 1)));
            t.add_css_class(&format!("mark-{}", m.key()));
            t.set_active(mark == Some(*m));
            let actions = Rc::clone(&self.actions);
            let m = *m;
            t.connect_clicked(move |b| {
                (actions.set_mark)(if b.is_active() { Some(m) } else { None });
            });
            marks.insert(&t, -1);
        }
        c.append(&marks);

        // Vídeos.
        if !d.videos.is_empty() {
            c.append(&heading(&format!("Vídeos ({})", d.videos.len())));
            for (i, v) in d.videos.iter().enumerate() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let b = gtk::Button::from_icon_name(if playing_video == Some(i) {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
                b.set_tooltip_text(Some("Escuchar"));
                let actions = Rc::clone(&self.actions);
                b.connect_clicked(move |_| (actions.play_video)(i));
                row.append(&b);
                let title = if v.title.is_empty() { &v.url } else { &v.title };
                let l = label(title, &[]);
                l.set_hexpand(true);
                row.append(&l);
                if v.duration > 0 {
                    let d = label(&format_secs(v.duration as f64), &["dim-label", "numeric"]);
                    d.set_wrap(false);
                    row.append(&d);
                }
                c.append(&row);
            }
        }

        // Tracklist.
        if !d.tracks.is_empty() {
            c.append(&heading("Tracklist"));
            let grid = gtk::Grid::builder()
                .column_spacing(10)
                .row_spacing(2)
                .build();
            for (i, t) in d.tracks.iter().enumerate() {
                let i = i as i32;
                grid.attach(&label(&t.pos, &["dim-label"]), 0, i, 1, 1);
                let title = label(&t.title, &[]);
                title.set_hexpand(true);
                grid.attach(&title, 1, i, 1, 1);
                let d = label(&t.duration, &["dim-label", "numeric"]);
                d.set_wrap(false);
                grid.attach(&d, 2, i, 1, 1);
            }
            c.append(&grid);
        }

        // Créditos.
        if !d.credits.is_empty() {
            c.append(&heading("Créditos"));
            for (role, name) in &d.credits {
                c.append(&label(&format!("{role}: {name}"), &["detail-fact"]));
            }
        }

        // Maletas.
        c.append(&heading("Maletas"));
        if suitcases.is_empty() {
            c.append(&label(
                "Aún no tienes maletas: créalas en el panel de la izquierda.",
                &["dim-label"],
            ));
        } else {
            let fb = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .max_children_per_line(3)
                .row_spacing(4)
                .column_spacing(4)
                .build();
            for s in suitcases {
                let t = gtk::ToggleButton::with_label(&s.name);
                t.add_css_class(&format!("suitcase-{}", s.color));
                t.set_active(in_suitcases.contains(&s.id));
                let actions = Rc::clone(&self.actions);
                let id = s.id;
                t.connect_toggled(move |b| (actions.set_suitcase)(id, b.is_active()));
                fb.insert(&t, -1);
            }
            c.append(&fb);
        }

        // Tu nota.
        c.append(&heading("Tu nota"));
        self.quiet.set(true);
        self.note.buffer().set_text(note);
        self.quiet.set(false);
        c.append(&self.note);

        // Notas de Discogs.
        if !d.notes.is_empty() {
            c.append(&heading("Notas de Discogs"));
            c.append(&label(&d.notes, &["detail-notes"]));
        }

        let open = gtk::LinkButton::with_label(&d.discogs_url(), "Abrir en Discogs");
        open.set_halign(gtk::Align::Start);
        open.set_margin_top(10);
        c.append(&open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duraciones() {
        assert_eq!(format_secs(372.9), "6:12");
        assert_eq!(format_secs(3723.0), "1:02:03");
        assert_eq!(format_secs(-3.0), "0:00");
    }
}
