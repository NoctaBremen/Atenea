//! Diálogo de importación (T07): elegir el `.xml.gz`, el nombre y los filtros (preajuste «Todo
//! en vinilo» por defecto; formato, géneros → estilos en árbol y años), y una barra de
//! progreso con discos/segundo y tiempo restante. La importación va en un hilo aparte y se
//! puede cancelar a mitad.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use vault::catalog::Catalog;
use vault::db::{ImportFilter, ImportOptions, Progress, import};

/// Formatos que se ofrecen.
const FORMATS: [&str; 6] = ["Vinyl", "CD", "Cassette", "File", "Shellac", "Box Set"];

/// Nombre de archivo a partir del nombre de la base: «Tech House 1995–2005» →
/// `tech-house-1995-2005`.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_owned();
    if s.is_empty() { "base".into() } else { s }
}

/// «quedan 3 min» a partir de la fracción hecha y el tiempo pasado.
pub fn eta(fraction: f64, elapsed: Duration) -> String {
    if fraction <= 0.001 {
        return "calculando…".into();
    }
    let left = elapsed.as_secs_f64() * (1.0 - fraction) / fraction;
    if left < 60.0 {
        format!("quedan {} s", left.ceil() as u64)
    } else if left < 3600.0 {
        format!("quedan {} min", (left / 60.0).ceil() as u64)
    } else {
        format!(
            "quedan {} h {} min",
            (left / 3600.0) as u64,
            ((left % 3600.0) / 60.0).ceil() as u64
        )
    }
}

/// Número con separador de miles (123.456).
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Un género del árbol, con sus estilos.
struct GenreNode {
    name: String,
    check: gtk::CheckButton,
    styles: Vec<(String, gtk::CheckButton)>,
}

/// Árbol género → estilos.
struct Tree {
    root: gtk::Box,
    nodes: Vec<Rc<GenreNode>>,
    syncing: Rc<Cell<bool>>,
}

impl Tree {
    fn new(catalog: &Catalog) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let syncing = Rc::new(Cell::new(false));
        let mut nodes = Vec::new();
        for (genre, styles) in &catalog.genres {
            let check = gtk::CheckButton::with_label(genre);
            let flow = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .max_children_per_line(3)
                .min_children_per_line(2)
                .column_spacing(6)
                .margin_start(24)
                .build();
            let styles: Vec<(String, gtk::CheckButton)> = styles
                .iter()
                .map(|s| {
                    let c = gtk::CheckButton::with_label(s);
                    flow.insert(&c, -1);
                    (s.clone(), c)
                })
                .collect();
            let node = Rc::new(GenreNode {
                name: genre.clone(),
                check,
                styles,
            });
            let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            header.append(&node.check);
            // El expansor solo hace de botón; los estilos van debajo, a todo el ancho.
            let exp = gtk::Expander::new(None);
            exp.set_tooltip_text(Some("Ver los estilos"));
            if !node.styles.is_empty() {
                header.append(&exp);
            }
            root.append(&header);
            root.append(&flow);
            flow.set_visible(false);
            exp.connect_expanded_notify(move |e| flow.set_visible(e.is_expanded()));

            // Género → todos sus estilos.
            {
                let (n, sync) = (Rc::clone(&node), Rc::clone(&syncing));
                node.check.connect_toggled(move |c| {
                    if sync.get() {
                        return;
                    }
                    sync.set(true);
                    c.set_inconsistent(false);
                    for (_, s) in &n.styles {
                        s.set_active(c.is_active());
                    }
                    sync.set(false);
                });
            }
            // Estilo → estado del género (todo, parte o nada).
            for (_, s) in &node.styles {
                let (n, sync) = (Rc::clone(&node), Rc::clone(&syncing));
                s.connect_toggled(move |_| {
                    if sync.get() {
                        return;
                    }
                    sync.set(true);
                    let on = n.styles.iter().filter(|(_, c)| c.is_active()).count();
                    n.check.set_active(on == n.styles.len());
                    n.check.set_inconsistent(on > 0 && on < n.styles.len());
                    sync.set(false);
                });
            }
            nodes.push(node);
        }
        Self {
            root,
            nodes,
            syncing,
        }
    }

    /// (géneros completos, pares género/estilo).
    fn selection(&self) -> (Vec<String>, Vec<(String, String)>) {
        let mut genres = Vec::new();
        let mut pairs = Vec::new();
        for n in &self.nodes {
            let on: Vec<&String> = n
                .styles
                .iter()
                .filter(|(_, c)| c.is_active())
                .map(|(s, _)| s)
                .collect();
            if n.check.is_active() && !n.check.is_inconsistent() {
                genres.push(n.name.clone());
            } else {
                pairs.extend(on.into_iter().map(|s| (n.name.clone(), s.clone())));
            }
        }
        (genres, pairs)
    }

    fn clear(&self) {
        self.syncing.set(true);
        for n in &self.nodes {
            n.check.set_active(false);
            n.check.set_inconsistent(false);
            for (_, s) in &n.styles {
                s.set_active(false);
            }
        }
        self.syncing.set(false);
    }
}

/// Abre el diálogo. `on_done` recibe la ruta de la base creada.
pub fn open(
    parent: &impl IsA<gtk::Window>,
    bases_dir: PathBuf,
    on_done: impl Fn(PathBuf) + 'static,
) {
    let win = gtk::Window::builder()
        .title("Importar dump de Discogs")
        .transient_for(parent)
        .modal(true)
        .default_width(640)
        .default_height(720)
        .build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for m in [
        gtk::Box::set_margin_start,
        gtk::Box::set_margin_end,
        gtk::Box::set_margin_top,
        gtk::Box::set_margin_bottom,
    ] {
        m(&outer, 14);
    }

    let grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(8)
        .build();
    let lab = |t: &str| {
        gtk::Label::builder()
            .label(t)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build()
    };
    let file_label = gtk::Label::builder()
        .label("Ningún archivo elegido")
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .hexpand(true)
        .build();
    let choose = gtk::Button::with_label("Elegir…");
    let file_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    file_box.append(&file_label);
    file_box.append(&choose);
    grid.attach(&lab("Dump"), 0, 0, 1, 1);
    grid.attach(&file_box, 1, 0, 1, 1);

    let name = gtk::Entry::builder()
        .text("Todo en vinilo")
        .hexpand(true)
        .build();
    grid.attach(&lab("Nombre"), 0, 1, 1, 1);
    grid.attach(&name, 1, 1, 1, 1);

    let preset = gtk::DropDown::from_strings(&["Todo en vinilo", "Personalizado"]);
    grid.attach(&lab("Preajuste"), 0, 2, 1, 1);
    grid.attach(&preset, 1, 2, 1, 1);

    let fmt_box = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(6)
        .build();
    let fmt_checks: Vec<(String, gtk::CheckButton)> = FORMATS
        .iter()
        .map(|f| {
            let c = gtk::CheckButton::with_label(f);
            c.set_active(*f == "Vinyl");
            fmt_box.insert(&c, -1);
            ((*f).to_owned(), c)
        })
        .collect();
    grid.attach(&lab("Formato"), 0, 3, 1, 1);
    grid.attach(&fmt_box, 1, 3, 1, 1);

    let years = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let from = gtk::SpinButton::with_range(0.0, 2100.0, 1.0);
    let to = gtk::SpinButton::with_range(0.0, 2100.0, 1.0);
    for s in [&from, &to] {
        s.set_tooltip_text(Some("0 = sin límite"));
    }
    years.append(&from);
    years.append(&gtk::Label::new(Some("–")));
    years.append(&to);
    years.append(&lab("(0 = sin límite)"));
    grid.attach(&lab("Años"), 0, 4, 1, 1);
    grid.attach(&years, 1, 4, 1, 1);

    let dest = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["dim-label"])
        .build();
    grid.attach(&lab("Se guarda en"), 0, 5, 1, 1);
    grid.attach(&dest, 1, 5, 1, 1);
    outer.append(&grid);

    outer.append(&lab(
        "Géneros y estilos (nada marcado = todos). Marca un género entero o despliégalo para elegir estilos.",
    ));
    let tree = Rc::new(Tree::new(&Catalog::builtin()));
    let scroll = gtk::ScrolledWindow::builder()
        .child(&tree.root)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    outer.append(&scroll);

    let progress = gtk::ProgressBar::builder()
        .show_text(true)
        .visible(false)
        .build();
    outer.append(&progress);
    let error = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["status-clip"])
        .visible(false)
        .build();
    outer.append(&error);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let start = gtk::Button::with_label("Importar");
    start.add_css_class("suggested-action");
    start.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&start);
    outer.append(&buttons);
    win.set_child(Some(&outer));

    let dump: Rc<RefCell<Option<PathBuf>>> = Rc::default();
    let running: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::default();
    let dest_path = {
        let (name, bases_dir) = (name.clone(), bases_dir.clone());
        move || bases_dir.join(format!("{}.db", slug(&name.text())))
    };
    let update_dest = {
        let (dest, dest_path) = (dest.clone(), dest_path.clone());
        move || dest.set_label(&dest_path().display().to_string())
    };
    update_dest();
    {
        let u = update_dest.clone();
        name.connect_changed(move |_| u());
    }

    // Preajuste: «Todo en vinilo» = solo vinilo, todos los géneros, sin años.
    let in_preset = Rc::new(Cell::new(false));
    {
        let (fmt, tree, from, to, name, in_preset) = (
            fmt_checks.clone(),
            Rc::clone(&tree),
            from.clone(),
            to.clone(),
            name.clone(),
            Rc::clone(&in_preset),
        );
        preset.connect_selected_notify(move |d| {
            if d.selected() == 0 {
                in_preset.set(true);
                for (f, c) in &fmt {
                    c.set_active(f == "Vinyl");
                }
                tree.clear();
                from.set_value(0.0);
                to.set_value(0.0);
                name.set_text("Todo en vinilo");
                in_preset.set(false);
            }
        });
    }
    // Tocar cualquier filtro pasa a «Personalizado».
    let to_custom = {
        let (preset, in_preset) = (preset.clone(), Rc::clone(&in_preset));
        move || {
            if !in_preset.get() {
                preset.set_selected(1);
            }
        }
    };
    for (_, c) in &fmt_checks {
        let f = to_custom.clone();
        c.connect_toggled(move |_| f());
    }
    for s in [&from, &to] {
        let f = to_custom.clone();
        s.connect_value_changed(move |_| f());
    }
    for n in &tree.nodes {
        let f = to_custom.clone();
        n.check.connect_toggled(move |_| f());
        for (_, c) in &n.styles {
            let f = to_custom.clone();
            c.connect_toggled(move |_| f());
        }
    }

    {
        let (win, dump, file_label, start) = (
            win.clone(),
            Rc::clone(&dump),
            file_label.clone(),
            start.clone(),
        );
        choose.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Dumps de Discogs (*.xml.gz, *.xml)"));
            filter.add_pattern("*.xml.gz");
            filter.add_pattern("*.xml");
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title("Elegir dump de releases")
                .filters(&filters)
                .build();
            let (dump, file_label, start) = (Rc::clone(&dump), file_label.clone(), start.clone());
            dialog.open(Some(&win), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res
                    && let Some(p) = f.path()
                {
                    file_label.set_label(&p.display().to_string());
                    dump.replace(Some(p));
                    start.set_sensitive(true);
                }
            });
        });
    }

    {
        let win2 = win.clone();
        let running = Rc::clone(&running);
        cancel.connect_clicked(move |_| {
            if let Some(flag) = running.borrow().as_ref() {
                flag.store(true, Ordering::Relaxed);
            } else {
                win2.close();
            }
        });
    }
    {
        let running = Rc::clone(&running);
        win.connect_close_request(move |_| {
            // Cerrar la ventana a mitad cancela la importación.
            if let Some(flag) = running.borrow().as_ref() {
                flag.store(true, Ordering::Relaxed);
            }
            glib::Propagation::Proceed
        });
    }

    let on_done = Rc::new(on_done);
    let win_c = win.clone();
    start.connect_clicked(move |start| {
        let Some(dump_path) = dump.borrow().clone() else {
            return;
        };
        let (genres, styles) = tree.selection();
        let year = |s: &gtk::SpinButton| {
            let v = s.value_as_int();
            (v > 0).then_some(v as u16)
        };
        let filter = ImportFilter {
            formats: fmt_checks
                .iter()
                .filter(|(_, c)| c.is_active())
                .map(|(f, _)| f.clone())
                .collect(),
            genres,
            styles,
            year_from: year(&from),
            year_to: year(&to),
        };
        let output = dest_path();
        let opts = ImportOptions {
            dump: dump_path,
            output: output.clone(),
            name: name.text().trim().to_owned(),
            filter,
        };
        let cancel_flag = Arc::new(AtomicBool::new(false));
        running.replace(Some(Arc::clone(&cancel_flag)));
        start.set_sensitive(false);
        grid.set_sensitive(false);
        scroll.set_sensitive(false);
        error.set_visible(false);
        progress.set_visible(true);
        progress.set_fraction(0.0);
        progress.set_text(Some("Empezando…"));

        let prog = Arc::new(Progress::default());
        let (tx, rx) = async_channel::bounded(1);
        {
            let prog = Arc::clone(&prog);
            let flag = Arc::clone(&cancel_flag);
            std::thread::spawn(move || {
                let r = import(&opts, &prog, &flag).map_err(|e| e.to_string());
                let _ = tx.send_blocking(r);
            });
        }
        let t0 = Instant::now();
        let tick = {
            let (progress, prog) = (progress.clone(), Arc::clone(&prog));
            glib::timeout_add_local(Duration::from_millis(250), move || {
                let f = prog.fraction();
                let seen = prog.seen.load(Ordering::Relaxed);
                let secs = t0.elapsed().as_secs_f64().max(0.001);
                progress.set_fraction(f);
                progress.set_text(Some(&format!(
                    "{:.0} % · {} leídos · {} importados · {} discos/s · {}",
                    f * 100.0,
                    thousands(seen),
                    thousands(prog.imported.load(Ordering::Relaxed)),
                    thousands((seen as f64 / secs) as u64),
                    eta(f, t0.elapsed())
                )));
                glib::ControlFlow::Continue
            })
        };
        let tick = Rc::new(RefCell::new(Some(tick)));
        let (win, running, start, grid, scroll, progress, error, on_done) = (
            win_c.clone(),
            Rc::clone(&running),
            start.clone(),
            grid.clone(),
            scroll.clone(),
            progress.clone(),
            error.clone(),
            Rc::clone(&on_done),
        );
        glib::spawn_future_local(async move {
            let result = rx
                .recv()
                .await
                .unwrap_or_else(|_| Err("el hilo de importación se cerró".into()));
            if let Some(t) = tick.take() {
                t.remove();
            }
            running.replace(None);
            match result {
                Ok(stats) => {
                    tracing::info!("importados {} de {}", stats.imported, stats.seen);
                    win.close();
                    on_done(output);
                }
                Err(e) => {
                    start.set_sensitive(true);
                    grid.set_sensitive(true);
                    scroll.set_sensitive(true);
                    progress.set_visible(false);
                    error.set_label(&e);
                    error.set_visible(true);
                }
            }
        });
    });

    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_y_textos() {
        assert_eq!(slug("Tech House 1995–2005"), "tech-house-1995-2005");
        assert_eq!(slug("Rock 70s"), "rock-70s");
        assert_eq!(slug("Música española"), "musica-espanola");
        assert_eq!(slug("  ¡¿!? "), "base");
        assert_eq!(thousands(1234567), "1.234.567");
        assert_eq!(thousands(12), "12");
        assert_eq!(eta(0.0, Duration::from_secs(5)), "calculando…");
        assert_eq!(eta(0.5, Duration::from_secs(30)), "quedan 30 s");
        assert_eq!(eta(0.25, Duration::from_secs(600)), "quedan 30 min");
        assert_eq!(eta(0.1, Duration::from_secs(3600)), "quedan 9 h 0 min");
    }
}
