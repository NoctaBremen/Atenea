//! Ventana principal: une la lista (T10), filtros (T11), detalle (T13), búsquedas guardadas
//! (T14), reproducción (T16), digging (T17), marcadores (T19), maletas (T20, T21), bases (T08)
//! y ajustes (T23).
//!
//! Regla de fluidez: las búsquedas van en su hilo ([`super::search`]); en el hilo principal
//! solo se leen páginas de 200 filas y el detalle de un disco (milisegundos).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use vault::config::Config;
use vault::db::base::DetailVideo;
use vault::db::{Base, Filtros, Mark, Row, Sort, SortKey};
use vault::player::{AudioServer, Player, PlayerEvent, PlayerOptions};
use vault::theme::{THEMES, Theme};
use vault::userdb::{self, SUITCASE_COLORS};

use super::detail::{DetailActions, DetailPanel};
use super::dialogs;
use super::filters::FilterPanel;
use super::model::{ReleaseModel, RowObject};
use super::playbar::PlayBar;
use super::search::{Request, Response, SearchWorker};

/// Estilo propio de Vault, después del CSS del tema.
const CSS: &str = "
.placeholder-title { font-size: 28pt; font-weight: bold; }
.settings-popover { padding: 12px; }
.section-title { font-size: 0.85em; font-weight: bold; }
.detail-artist { font-size: 1.15em; font-weight: bold; }
.detail-title { font-size: 1.05em; margin-bottom: 4px; }
.detail-fact { font-size: 0.95em; }
.detail-notes { font-size: 0.9em; }
.base-title { font-weight: bold; }
.base-subtitle { font-size: 0.85em; }
.statusbar { padding: 2px 8px; font-size: 0.9em; }
.playbar { border-top: 1px solid @at_rejilla; }
.note { border: 1px solid @at_rejilla; padding: 4px; }
.mark-dot { font-size: 1.1em; }
.playing-dot { color: @at_picos; }
columnview > listview > row:selected .playing-dot,
columnview > listview > row:selected label { color: @at_grafica; }
.mark-interesa { color: @at_ok; }
.mark-no_interesa { color: @at_clip; }
.mark-escuchado { color: @at_aviso; }
.mark-volver { color: @at_media; }
togglebutton.mark-interesa:checked { background-color: @at_ok; color: @at_grafica; }
togglebutton.mark-no_interesa:checked { background-color: @at_clip; color: @at_grafica; }
togglebutton.mark-escuchado:checked { background-color: @at_aviso; color: @at_grafica; }
togglebutton.mark-volver:checked { background-color: @at_media; color: @at_grafica; }
.suitcase-curva { color: @at_curva; }
.suitcase-media { color: @at_media; }
.suitcase-picos { color: @at_picos; }
.suitcase-ok { color: @at_ok; }
.suitcase-aviso { color: @at_aviso; }
.suitcase-clip { color: @at_clip; }
togglebutton.suitcase-curva:checked { background-color: @at_curva; color: @at_grafica; }
togglebutton.suitcase-media:checked { background-color: @at_media; color: @at_grafica; }
togglebutton.suitcase-picos:checked { background-color: @at_picos; color: @at_grafica; }
togglebutton.suitcase-ok:checked { background-color: @at_ok; color: @at_grafica; }
togglebutton.suitcase-aviso:checked { background-color: @at_aviso; color: @at_grafica; }
togglebutton.suitcase-clip:checked { background-color: @at_clip; color: @at_grafica; }
.drop-hover { background-color: @at_rejilla; }
";

/// CSS completo de la ventana para un tema ya ajustado.
pub fn css_for(theme: &Theme) -> String {
    format!("{}{CSS}", theme.gtk_css())
}

/// Índice del tema con ese id en [`THEMES`]; si no existe, el de por defecto (0).
pub fn theme_index(id: &str) -> usize {
    THEMES.iter().position(|t| t.id == id).unwrap_or(0)
}

/// Lo que suena.
struct NowPlaying {
    release_id: i64,
    label: String,
    videos: Vec<DetailVideo>,
    index: usize,
}

/// Estado de la ventana.
pub struct App {
    window: gtk::ApplicationWindow,
    config: RefCell<Config>,
    css: gtk::CssProvider,
    user_path: Option<PathBuf>,
    base: RefCell<Option<Rc<Base>>>,
    model: ReleaseModel,
    selection: gtk::SingleSelection,
    view: gtk::ColumnView,
    status: gtk::Label,
    base_title: gtk::Label,
    base_subtitle: gtk::Label,
    filters: Rc<FilterPanel>,
    detail: Rc<DetailPanel>,
    playbar: PlayBar,
    suitcase_list: gtk::ListBox,
    search: RefCell<Option<SearchWorker>>,
    generation: Cell<u64>,
    debounce: RefCell<Option<glib::SourceId>>,
    sort: Cell<Sort>,
    player: RefCell<Option<Player>>,
    player_events: RefCell<Option<Receiver<PlayerEvent>>>,
    now: RefCell<Option<NowPlaying>>,
    center: gtk::Stack,
    /// Precarga en curso de la base abierta (se cancela al cambiar de base).
    warming: RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    /// Búsqueda guardada cargada: (id, nombre, filtros, orden) tal como se guardó.
    active_search: RefCell<Option<(i64, String, Filtros, Sort)>>,
    /// Disco que hay que elegir cuando lleguen los resultados (al restaurar la sesión).
    pending_select: Cell<Option<i64>>,
    /// Guardado diferido de la sesión al moverse por la lista.
    session_timer: RefCell<Option<glib::SourceId>>,
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build()
}

impl App {
    fn filters(&self) -> Rc<FilterPanel> {
        Rc::clone(&self.filters)
    }

    fn detail(&self) -> Rc<DetailPanel> {
        Rc::clone(&self.detail)
    }

    fn base(&self) -> Option<Rc<Base>> {
        self.base.borrow().clone()
    }

    fn set_status(&self, msg: &str) {
        self.status.set_label(msg);
    }

    // ---------- Bases (T08) ----------

    /// Abre una base y lanza la primera búsqueda.
    fn open_base(self: &Rc<Self>, path: &Path) {
        match Base::open(path, self.user_path.as_deref()) {
            Ok(b) => {
                let b = Rc::new(b);
                let info = b.info().clone();
                self.base_title.set_label(&info.name);
                let mut sub = format!(
                    "{} discos",
                    super::import::thousands(info.release_count.max(0) as u64)
                );
                if !info.dump_date.is_empty() {
                    sub.push_str(&format!(" · dump del {}", info.dump_date));
                }
                if !info.filter_text.is_empty() {
                    sub.push_str(&format!(" · {}", info.filter_text));
                }
                self.base_subtitle.set_label(&sub);
                self.base_subtitle.set_visible(true);
                self.window
                    .set_title(Some(&format!("Vault — {}", info.name)));
                self.model.set_base(Some(Rc::clone(&b)));
                self.filters().load_base(&b);
                self.base.replace(Some(b));
                self.start_warming(path);
                self.reload_suitcases();
                {
                    let mut c = self.config.borrow_mut();
                    c.push_recent(path);
                    c.save();
                }
                self.center.set_visible_child_name("list");
                self.detail().show_empty("Elige un disco de la lista.");
                self.restore_session(path);
                self.run_search();
            }
            Err(e) => {
                self.config.borrow_mut().forget_recent(path);
                self.config.borrow().save();
                dialogs::inform(&self.window, "No se pudo abrir la base", &e.to_string());
            }
        }
    }

    /// Precarga la base en la caché del sistema en un hilo aparte si cabe holgadamente en la
    /// RAM libre, para que las búsquedas lean de memoria desde el principio.
    fn start_warming(&self, path: &Path) {
        if let Some(old) = self.warming.take() {
            old.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if !vault::memory::plan_for(path).warm {
            return;
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.warming.replace(Some(std::sync::Arc::clone(&cancel)));
        let path = path.to_owned();
        let spawned = std::thread::Builder::new()
            .name("vault-precarga".into())
            .spawn(move || {
                let t = std::time::Instant::now();
                match vault::memory::warm_file(&path, &cancel) {
                    Ok(n) => tracing::info!(
                        "base precargada en memoria: {} MB en {:.1} s",
                        n >> 20,
                        t.elapsed().as_secs_f64()
                    ),
                    Err(e) => tracing::warn!("no se pudo precargar la base: {e}"),
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("no se pudo lanzar la precarga: {e}");
        }
    }

    fn choose_base(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Bases de Vault (*.db)"));
        filter.add_pattern("*.db");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Abrir base")
            .filters(&filters)
            .build();
        if let Some(dir) = self.config.borrow().bases_dir() {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(dir)));
        }
        let this = Rc::clone(self);
        dialog.open(Some(&self.window), gtk::gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r
                && let Some(p) = f.path()
            {
                this.open_base(&p);
            }
        });
    }

    fn import_dialog(self: &Rc<Self>) {
        let dir = self
            .config
            .borrow()
            .bases_dir()
            .unwrap_or_else(|| PathBuf::from("."));
        let this = Rc::clone(self);
        super::import::open(&self.window, dir, move |p| this.open_base(&p));
    }

    // ---------- Sesión: volver donde lo dejaste ----------

    /// Pone los filtros, el orden, la búsqueda guardada y el disco de la última sesión con
    /// esta base. Con otra base, empieza sin filtros.
    fn restore_session(&self, path: &Path) {
        let session = self
            .config
            .borrow()
            .session
            .clone()
            .filter(|s| s.base == path);
        self.active_search.replace(None);
        let Some(s) = session else {
            self.filters().set(&Filtros::default());
            self.apply_sort(Sort::default());
            return;
        };
        self.filters().set(&s.filtros);
        self.apply_sort(s.sort);
        self.pending_select.set(s.selected);
        if let (Some(id), Some(base)) = (s.saved_search, self.base()) {
            let found = base
                .user()
                .saved_searches()
                .unwrap_or_default()
                .into_iter()
                .find(|x| x.id == id);
            if let Some(x) = found {
                self.active_search
                    .replace(Some((x.id, x.name, x.filtros, x.sort)));
            }
        }
    }

    /// Guarda en la configuración dónde estás (filtros, orden, disco, búsqueda guardada).
    fn save_session(&self) {
        let Some(base) = self.base() else { return };
        let session = vault::config::Session {
            base: base.info().path.clone(),
            filtros: self.filters().get(),
            sort: self.sort.get(),
            selected: self.selected(),
            saved_search: self.active_search.borrow().as_ref().map(|a| a.0),
        };
        let mut c = self.config.borrow_mut();
        if c.session.as_ref() != Some(&session) {
            c.session = Some(session);
            c.save();
        }
    }

    /// Guarda la sesión un segundo después del último movimiento por la lista.
    fn save_session_later(self: &Rc<Self>) {
        if let Some(t) = self.session_timer.take() {
            t.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(1), move || {
            if let Some(a) = weak.upgrade() {
                a.session_timer.replace(None);
                a.save_session();
            }
        });
        self.session_timer.replace(Some(id));
    }

    /// Pone el orden en la cabecera de la lista (y en `self.sort`). Cambiar el orden de la
    /// cabecera lanza una búsqueda por su cuenta.
    fn apply_sort(&self, sort: Sort) {
        self.sort.set(sort);
        let id = match sort.key {
            SortKey::Artista => "artista",
            SortKey::Titulo => "titulo",
            SortKey::Sello => "sello",
            SortKey::Catno => "catno",
            SortKey::Anyo => "anyo",
            SortKey::Pais => "pais",
            SortKey::Id => "",
        };
        let cols = self.view.columns();
        let col = (0..cols.n_items())
            .filter_map(|i| cols.item(i).and_downcast::<gtk::ColumnViewColumn>())
            .find(|c| c.id().is_some_and(|x| x == id));
        let order = if sort.desc {
            gtk::SortType::Descending
        } else {
            gtk::SortType::Ascending
        };
        if sort == Sort::default() {
            self.view
                .sort_by_column(None::<&gtk::ColumnViewColumn>, order);
        } else {
            self.view.sort_by_column(col.as_ref(), order);
        }
    }

    /// Texto de la búsqueda guardada activa para la barra de estado.
    fn active_search_label(&self) -> Option<String> {
        let a = self.active_search.borrow();
        let (_, name, filtros, sort) = a.as_ref()?;
        let modified = self.filters().get() != *filtros || self.sort.get() != *sort;
        Some(if modified {
            format!("«{name}» (modificada)")
        } else {
            format!("«{name}»")
        })
    }

    // ---------- Búsqueda (T09–T12) ----------

    fn schedule_search(self: &Rc<Self>) {
        if let Some(t) = self.debounce.take() {
            t.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(this) = weak.upgrade() {
                this.debounce.replace(None);
                this.run_search();
            }
        });
        self.debounce.replace(Some(id));
    }

    fn run_search(&self) {
        let Some(base) = self.base() else { return };
        let g = self.generation.get() + 1;
        self.generation.set(g);
        self.set_status("Buscando…");
        if let Some(w) = self.search.borrow().as_ref() {
            w.request(Request {
                generation: g,
                base: base.info().path.clone(),
                user: self.user_path.clone(),
                filtros: self.filters().get(),
                sort: self.sort.get(),
            });
        }
    }

    fn on_results(self: &Rc<Self>, r: Response) {
        if r.generation != self.generation.get() {
            return;
        }
        match r.result {
            Ok(ids) => {
                let n = ids.len();
                // El disco a dejar elegido: el de la sesión anterior (al abrir) o el que
                // estaba en el detalle, si siguen en la lista.
                let restoring = self.pending_select.take();
                let keep = restoring.or(self.detail().current());
                self.model.set_ids(ids);
                match keep.and_then(|id| self.model.position_of(id)) {
                    // Al restaurar la sesión se lleva la lista hasta el disco; si no, se elige
                    // sin mover el foco (el usuario puede estar escribiendo en el buscador).
                    Some(pos) if restoring.is_some() => {
                        self.selection.set_selected(pos);
                        // Llevar la lista hasta el disco cuando ya esté dibujada: hacerlo antes
                        // dejaba filas en blanco.
                        let weak = Rc::downgrade(self);
                        glib::idle_add_local_once(move || {
                            if let Some(a) = weak.upgrade()
                                && a.selection.selected() == pos
                            {
                                a.select_position(pos);
                            }
                        });
                    }
                    Some(pos) => self.selection.set_selected(pos),
                    None => self.selection.set_selected(gtk::INVALID_LIST_POSITION),
                }
                let mut status = format!(
                    "{} discos · {} ms",
                    super::import::thousands(n as u64),
                    r.elapsed.as_millis()
                );
                if let Some(name) = self.active_search_label() {
                    status = format!("{name} · {status}");
                }
                self.set_status(&status);
                self.save_session();
            }
            Err(e) => self.set_status(&format!("Error en la búsqueda: {e}")),
        }
    }

    fn on_sort_changed(self: &Rc<Self>) {
        let Some(sorter) = self
            .view
            .sorter()
            .and_then(|s| s.downcast::<gtk::ColumnViewSorter>().ok())
        else {
            return;
        };
        let key = sorter
            .primary_sort_column()
            .and_then(|c| c.id())
            .and_then(|id| match id.as_str() {
                "artista" => Some(SortKey::Artista),
                "titulo" => Some(SortKey::Titulo),
                "sello" => Some(SortKey::Sello),
                "catno" => Some(SortKey::Catno),
                "anyo" => Some(SortKey::Anyo),
                "pais" => Some(SortKey::Pais),
                _ => None,
            })
            .unwrap_or_default();
        let desc = sorter.primary_sort_order() == gtk::SortType::Descending;
        self.sort.set(Sort { key, desc });
        self.run_search();
    }

    // ---------- Selección y detalle (T13) ----------

    fn selected(&self) -> Option<i64> {
        self.selection
            .selected_item()
            .and_downcast::<RowObject>()
            .map(|o| o.id())
    }

    fn show_detail(self: &Rc<Self>) {
        let Some(base) = self.base() else { return };
        let Some(id) = self.selected() else {
            return;
        };
        self.save_session_later();
        let u = base.user();
        match base.detail(id) {
            Ok(Some(d)) => {
                let playing = self
                    .now
                    .borrow()
                    .as_ref()
                    .filter(|n| n.release_id == id)
                    .map(|n| n.index);
                self.detail().show(
                    &d,
                    u.mark(id).ok().flatten(),
                    &u.note(id).unwrap_or_default(),
                    &u.suitcases().unwrap_or_default(),
                    &u.suitcases_of(id).unwrap_or_default(),
                    playing,
                );
            }
            Ok(None) => self.detail().show_empty("Este disco no está en la base."),
            Err(e) => self.detail().show_empty(&format!("Error: {e}")),
        }
    }

    fn select_position(&self, pos: u32) {
        self.selection.set_selected(pos);
        self.view.scroll_to(
            pos,
            None::<&gtk::ColumnViewColumn>,
            gtk::ListScrollFlags::FOCUS,
            None,
        );
    }

    // ---------- Marcadores (T19) ----------

    fn set_mark(self: &Rc<Self>, id: i64, mark: Option<Mark>) {
        let Some(base) = self.base() else { return };
        if let Err(e) = base.user().set_mark(id, mark) {
            self.set_status(&e.to_string());
            return;
        }
        self.repaint_row(|| self.model.set_mark(id, mark));
        if self.detail().current() == Some(id) {
            self.show_detail();
        }
    }

    /// Repinta la fila de un disco sin perder la selección ni el foco: `items_changed` sobre
    /// la fila elegida la deselecciona y recrea su widget, así que se vuelven a poner los dos
    /// para que las flechas y los atajos sigan donde estaban.
    fn repaint_row(&self, repaint: impl FnOnce()) {
        let sel = self.selection.selected();
        let had_focus = gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|f| {
            f == *self.view.upcast_ref::<gtk::Widget>() || f.is_ancestor(&self.view)
        });
        repaint();
        if sel != gtk::INVALID_LIST_POSITION && sel < self.model.n_items() {
            if self.selection.selected() != sel {
                self.selection.set_selected(sel);
            }
            if had_focus {
                self.view.scroll_to(
                    sel,
                    None::<&gtk::ColumnViewColumn>,
                    gtk::ListScrollFlags::FOCUS,
                    None,
                );
            }
        }
    }

    // ---------- Maletas (T20, T21) ----------

    fn reload_suitcases(self: &Rc<Self>) {
        let list = &self.suitcase_list;
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        let Some(base) = self.base() else { return };
        let suitcases = base.user().suitcases().unwrap_or_default();
        self.filters().load_suitcases(&suitcases);
        for (i, s) in suitcases.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_margin_start(6);
            row.set_margin_end(4);
            let dot = gtk::Label::new(Some("●"));
            dot.add_css_class(&format!("suitcase-{}", s.color));
            row.append(&dot);
            let name = gtk::Label::builder()
                .label(&s.name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            if i < 9 {
                name.set_tooltip_text(Some(&format!(
                    "Alt+{}: meter el disco elegido · arrastra discos aquí",
                    i + 1
                )));
            }
            row.append(&name);
            row.append(&dim(&s.count.to_string()));
            let menu = gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .css_classes(["flat"])
                .build();
            menu.set_popover(Some(&self.suitcase_popover(s.id, &s.name)));
            row.append(&menu);

            // Soltar un disco arrastrado desde la lista.
            let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
            {
                let (this, sid, r) = (Rc::clone(self), s.id, row.clone());
                drop.connect_drop(move |_, value, _, _| {
                    r.remove_css_class("drop-hover");
                    let Some(id) = value.get::<String>().ok().and_then(|s| {
                        s.strip_prefix("vault-release:")
                            .and_then(|x| x.parse().ok())
                    }) else {
                        return false;
                    };
                    this.add_to_suitcase(sid, id);
                    true
                });
            }
            {
                let r = row.clone();
                drop.connect_enter(move |_, _, _| {
                    r.add_css_class("drop-hover");
                    gdk::DragAction::COPY
                });
            }
            {
                let r = row.clone();
                drop.connect_leave(move |_| r.remove_css_class("drop-hover"));
            }
            row.add_controller(drop);

            let lrow = gtk::ListBoxRow::builder().child(&row).build();
            lrow.set_widget_name(&s.id.to_string());
            list.append(&lrow);
        }
    }

    fn suitcase_popover(self: &Rc<Self>, id: i64, name: &str) -> gtk::Popover {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
        b.add_css_class("settings-popover");
        let pop = gtk::Popover::builder().child(&b).build();
        let view = gtk::Button::with_label("Ver la maleta");
        let rename = gtk::Button::with_label("Renombrar…");
        let export = gtk::Button::with_label("Exportar CSV…");
        let delete = gtk::Button::with_label("Borrar…");
        delete.add_css_class("destructive-action");
        for w in [&view, &rename, &export] {
            w.add_css_class("flat");
            b.append(w);
        }
        let colors = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        for c in SUITCASE_COLORS {
            let btn = gtk::Button::with_label("●");
            btn.add_css_class("flat");
            btn.add_css_class(&format!("suitcase-{c}"));
            let (this, pop) = (Rc::clone(self), pop.clone());
            btn.connect_clicked(move |_| {
                pop.popdown();
                if let Some(base) = this.base() {
                    let _ = base.user().set_suitcase_color(id, c);
                }
                this.reload_suitcases_later();
            });
            colors.append(&btn);
        }
        b.append(&colors);
        b.append(&delete);

        {
            let (this, pop) = (Rc::clone(self), pop.clone());
            view.connect_clicked(move |_| {
                pop.popdown();
                this.filters().select_suitcase(Some(id));
            });
        }
        {
            let (this, pop, name) = (Rc::clone(self), pop.clone(), name.to_owned());
            rename.connect_clicked(move |_| {
                pop.popdown();
                let t = Rc::clone(&this);
                dialogs::ask_text(
                    &this.window,
                    "Renombrar maleta",
                    &name,
                    "Renombrar",
                    move |n| {
                        let base = t.base().ok_or("No hay base abierta")?;
                        base.user()
                            .rename_suitcase(id, &n)
                            .map_err(|e| e.to_string())?;
                        t.reload_suitcases_later();
                        Ok(())
                    },
                );
            });
        }
        {
            let (this, pop, name) = (Rc::clone(self), pop.clone(), name.to_owned());
            export.connect_clicked(move |_| {
                pop.popdown();
                this.export_suitcase(id, &name);
            });
        }
        {
            let (this, pop, name) = (Rc::clone(self), pop.clone(), name.to_owned());
            delete.connect_clicked(move |_| {
                pop.popdown();
                let t = Rc::clone(&this);
                dialogs::confirm(
                    &this.window,
                    &format!("¿Borrar la maleta «{name}»?"),
                    "Los discos no se borran, solo la maleta.",
                    "Borrar",
                    move || {
                        if let Some(base) = t.base() {
                            let _ = base.user().delete_suitcase(id);
                        }
                        t.reload_suitcases_later();
                        t.schedule_search();
                    },
                );
            });
        }
        pop
    }

    /// Recarga las maletas cuando acabe el manejador actual (no se puede quitar un widget
    /// desde su propia señal).
    fn reload_suitcases_later(self: &Rc<Self>) {
        let this = Rc::clone(self);
        glib::idle_add_local_once(move || {
            this.reload_suitcases();
            if this.detail().current().is_some() {
                this.show_detail();
            }
        });
    }

    fn new_suitcase(self: &Rc<Self>) {
        if self.base().is_none() {
            dialogs::inform(&self.window, "Abre una base primero", "");
            return;
        }
        let this = Rc::clone(self);
        dialogs::ask_text(&self.window, "Nueva maleta", "", "Crear", move |n| {
            let base = this.base().ok_or("No hay base abierta")?;
            let count = base.user().suitcases().map(|s| s.len()).unwrap_or(0);
            let color = SUITCASE_COLORS[count % SUITCASE_COLORS.len()];
            base.user()
                .create_suitcase(&n, color)
                .map_err(|e| e.to_string())?;
            this.reload_suitcases_later();
            Ok(())
        });
    }

    fn add_to_suitcase(self: &Rc<Self>, suitcase: i64, release: i64) {
        let Some(base) = self.base() else { return };
        match base.user().add_to_suitcase(suitcase, release) {
            Ok(()) => {
                let name = base
                    .user()
                    .suitcases()
                    .ok()
                    .and_then(|v| v.into_iter().find(|s| s.id == suitcase))
                    .map(|s| s.name)
                    .unwrap_or_default();
                self.set_status(&format!("Añadido a «{name}»"));
                self.reload_suitcases_later();
            }
            Err(e) => self.set_status(&e.to_string()),
        }
    }

    fn export_suitcase(self: &Rc<Self>, id: i64, name: &str) {
        let Some(base) = self.base() else { return };
        let ids = base.user().suitcase_items(id).unwrap_or_default();
        let rows: Vec<Row> = base.rows(&ids).unwrap_or_default();
        let missing = ids.len() - rows.len();
        let dialog = gtk::FileDialog::builder()
            .title("Exportar maleta")
            .initial_name(format!("{}.csv", super::import::slug(name)))
            .build();
        let this = Rc::clone(self);
        dialog.save(Some(&self.window), gtk::gio::Cancellable::NONE, move |r| {
            let Ok(f) = r else { return };
            let Some(p) = f.path() else { return };
            let res = std::fs::File::create(&p)
                .and_then(|file| vault::export::write_csv(std::io::BufWriter::new(file), &rows));
            match res {
                Ok(()) => {
                    let mut msg = format!("{} discos exportados a {}", rows.len(), p.display());
                    if missing > 0 {
                        msg.push_str(&format!(" ({missing} no están en esta base)"));
                    }
                    this.set_status(&msg);
                }
                Err(e) => dialogs::inform(&this.window, "No se pudo exportar", &e.to_string()),
            }
        });
    }

    // ---------- Reproducción (T15–T17) ----------

    fn ensure_player(self: &Rc<Self>) -> bool {
        if self.player.borrow().is_some() {
            return true;
        }
        let c = self.config.borrow().clone();
        let opts = PlayerOptions {
            audio: c.audio,
            device: c.audio_device.clone(),
            video: c.video,
            ao_override: std::env::var("VAULT_AO").ok(),
        };
        match Player::launch(&opts) {
            Ok(p) => {
                let _ = p.set_volume(c.volume);
                self.player_events.replace(p.take_events());
                self.player.replace(Some(p));
                true
            }
            Err(e) => {
                dialogs::inform(&self.window, "No se puede reproducir", &e.to_string());
                false
            }
        }
    }

    /// Reproduce el vídeo `index` de un disco.
    fn play(self: &Rc<Self>, release_id: i64, index: usize) {
        let Some(base) = self.base() else { return };
        let Ok(Some(d)) = base.detail(release_id) else {
            return;
        };
        if d.videos.is_empty() {
            self.set_status("Este disco no tiene vídeos.");
            if self.playbar.digging.is_active() {
                self.next_release();
            }
            return;
        }
        let index = index.min(d.videos.len() - 1);
        if !self.ensure_player() {
            return;
        }
        let url = d.videos[index].url.clone();
        let label = format!("{} — {}", d.row.artist, d.row.title);
        if let Some(p) = self.player.borrow().as_ref()
            && let Err(e) = p.play(&url)
        {
            self.set_status(&e.to_string());
            return;
        }
        let old_id = self.now.borrow().as_ref().map(|n| n.release_id);
        let vt = d.videos[index].title.clone();
        self.playbar.set_title(&format!(
            "{label} · {} ({}/{})",
            if vt.is_empty() { &url } else { &vt },
            index + 1,
            d.videos.len()
        ));
        self.playbar.set_paused(false);
        self.now.replace(Some(NowPlaying {
            release_id,
            label,
            videos: d.videos,
            index,
        }));
        if self.detail().current() == Some(release_id) {
            self.show_detail();
        }
        self.repaint_row(|| {
            if let Some(old) = old_id
                && old != release_id
            {
                self.model.refresh_row(old);
            }
            self.model.refresh_row(release_id);
        });
    }

    fn play_selected(self: &Rc<Self>) {
        if let Some(id) = self.selected() {
            self.play(id, 0);
        }
    }

    fn toggle_pause(self: &Rc<Self>) {
        if self.now.borrow().is_none() {
            self.play_selected();
            return;
        }
        if let Some(p) = self.player.borrow().as_ref() {
            let _ = p.toggle_pause();
        }
    }

    fn seek(&self, secs: f64) {
        if let Some(p) = self.player.borrow().as_ref() {
            let _ = p.seek_relative(secs);
        }
    }

    /// Vídeo siguiente (`+1`) o anterior (`-1`) del disco que suena.
    fn step_video(self: &Rc<Self>, delta: i64) -> bool {
        let target = self.now.borrow().as_ref().and_then(|n| {
            let i = n.index as i64 + delta;
            (i >= 0 && (i as usize) < n.videos.len()).then_some((n.release_id, i as usize))
        });
        match target {
            Some((id, i)) => {
                self.play(id, i);
                true
            }
            None => false,
        }
    }

    /// Siguiente disco de la lista (digging): lo selecciona y lo pone.
    fn next_release(self: &Rc<Self>) {
        self.step_release(1);
    }

    fn step_release(self: &Rc<Self>, delta: i64) {
        let n = self.model.n_items();
        if n == 0 {
            return;
        }
        let cur = self.selection.selected();
        let pos = if cur == gtk::INVALID_LIST_POSITION {
            0
        } else {
            (i64::from(cur) + delta).clamp(0, i64::from(n) - 1) as u32
        };
        if pos == cur && delta != 0 {
            self.set_status("Fin de la lista.");
            return;
        }
        self.select_position(pos);
        if self.playbar.digging.is_active() || self.now.borrow().is_some() {
            self.play_selected();
        }
    }

    fn poll_player(self: &Rc<Self>) {
        let events: Vec<PlayerEvent> = match self.player_events.borrow().as_ref() {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for e in events {
            match e {
                PlayerEvent::Position(p) => self.playbar.set_position(p),
                PlayerEvent::Duration(d) => self.playbar.set_duration(d),
                PlayerEvent::Paused(p) => self.playbar.set_paused(p),
                PlayerEvent::Volume(_) | PlayerEvent::Title(_) | PlayerEvent::Loading => {}
                PlayerEvent::Ended(reason) => {
                    if reason == "error" {
                        let what = self.now.borrow().as_ref().map(|n| n.label.clone());
                        self.set_status(&format!(
                            "No se pudo reproducir un vídeo de {} (¿caído?); paso al siguiente.",
                            what.unwrap_or_default()
                        ));
                    }
                    if (reason == "eof" || reason == "error") && !self.step_video(1) {
                        if self.playbar.digging.is_active() {
                            self.next_release();
                        } else {
                            self.stop_playback();
                        }
                    }
                }
                PlayerEvent::Idle => {}
                PlayerEvent::Exited => {
                    self.player.replace(None);
                    self.player_events.replace(None);
                    self.stop_playback();
                    break;
                }
            }
        }
    }

    fn stop_playback(&self) {
        let id = self.now.take().map(|n| n.release_id);
        self.playbar.reset();
        if let Some(id) = id {
            self.repaint_row(|| self.model.refresh_row(id));
        }
    }

    fn restart_player(&self) {
        // Al cambiar el sonido se cierra mpv; el siguiente play lo lanza con los ajustes nuevos.
        self.player_events.replace(None);
        self.player.replace(None);
        self.stop_playback();
    }

    // ---------- Columnas ----------

    /// Guarda en la configuración el orden, ancho y visibilidad de las columnas.
    fn save_columns(&self) {
        let cols = self.view.columns();
        let mut state = Vec::new();
        for i in 0..cols.n_items() {
            let Some(c) = cols.item(i).and_downcast::<gtk::ColumnViewColumn>() else {
                continue;
            };
            let Some(id) = c.id() else { continue };
            // Todas empiezan con ancho fijo y al estirarlas GTK actualiza ese ancho.
            let width = if c.fixed_width() > 0 {
                c.fixed_width()
            } else {
                COLUMNS.iter().find(|x| x.0 == id).map_or(100, |x| x.2)
            };
            state.push(vault::config::ColumnState {
                id: id.to_string(),
                visible: c.is_visible(),
                width,
            });
        }
        let mut cfg = self.config.borrow_mut();
        cfg.columns = state;
        cfg.save();
    }

    // ---------- Teclado ----------

    fn on_key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        // Con el foco en un campo de texto, las teclas son para escribir.
        if let Some(f) = gtk::prelude::GtkWindowExt::focus(&self.window)
            && (f.is::<gtk::Text>() || f.is::<gtk::TextView>() || f.is::<gtk::SpinButton>())
        {
            if key == gdk::Key::Escape {
                self.view.grab_focus();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        if ctrl && key == gdk::Key::f {
            self.filters().texto.grab_focus();
            return glib::Propagation::Stop;
        }
        if ctrl || state.contains(gdk::ModifierType::SUPER_MASK) {
            return glib::Propagation::Proceed;
        }
        let digit = key.to_unicode().and_then(|c| c.to_digit(10));
        if alt {
            // Alt+1..9: meter el disco elegido en la maleta n.
            if let (Some(n @ 1..=9), Some(id)) = (digit, self.selected()) {
                let s = self
                    .base()
                    .and_then(|b| b.user().suitcases().ok())
                    .and_then(|v| v.get(n as usize - 1).map(|s| s.id));
                if let Some(sid) = s {
                    self.add_to_suitcase(sid, id);
                }
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::space => self.toggle_pause(),
            gdk::Key::Left => self.seek(-10.0),
            gdk::Key::Right => self.seek(10.0),
            gdk::Key::comma => {
                self.step_video(-1);
            }
            gdk::Key::period => {
                self.step_video(1);
            }
            gdk::Key::n | gdk::Key::N => self.step_release(1),
            gdk::Key::p | gdk::Key::P => self.step_release(-1),
            gdk::Key::d | gdk::Key::D => self
                .playbar
                .digging
                .set_active(!self.playbar.digging.is_active()),
            gdk::Key::v | gdk::Key::V => self
                .playbar
                .video
                .set_active(!self.playbar.video.is_active()),
            _ => match digit {
                Some(n @ 1..=4) => {
                    let Some(id) = self.selected() else {
                        return glib::Propagation::Stop;
                    };
                    let m = Mark::ALL[n as usize - 1];
                    let current = self.base().and_then(|b| b.user().mark(id).ok().flatten());
                    // Pulsar la tecla del marcador que ya tiene lo quita.
                    let new = if current == Some(m) { None } else { Some(m) };
                    self.set_mark(id, new);
                    if self.playbar.digging.is_active() && new.is_some() {
                        self.next_release();
                    }
                }
                Some(0) => {
                    if let Some(id) = self.selected() {
                        self.set_mark(id, None);
                    }
                }
                _ => return glib::Propagation::Proceed,
            },
        }
        glib::Propagation::Stop
    }
}

// ---------- Columnas ----------

fn text_column(
    app: &std::rc::Weak<App>,
    id: &str,
    title: &str,
    width: i32,
    get: fn(&Row) -> String,
) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    let drag_source = id == "artista" || id == "titulo";
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let l = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        if drag_source {
            // Arrastrar un disco a una maleta.
            let src = gtk::DragSource::new();
            src.set_actions(gdk::DragAction::COPY);
            let item_w = item.downgrade();
            src.connect_prepare(move |_, _, _| {
                let item = item_w.upgrade()?;
                let obj = item.item().and_downcast::<RowObject>()?;
                Some(gdk::ContentProvider::for_value(
                    &format!("vault-release:{}", obj.id()).to_value(),
                ))
            });
            l.add_controller(src);
        }
        item.set_child(Some(&l));
    });
    let app_w = app.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let (Some(obj), Some(l)) = (
            item.item().and_downcast::<RowObject>(),
            item.child().and_downcast::<gtk::Label>(),
        ) else {
            return;
        };
        let text = get(&obj.row());
        l.set_tooltip_text(if text.len() > 30 {
            Some(text.as_str())
        } else {
            None
        });
        l.set_label(&text);
        let playing = app_w.upgrade().is_some_and(|a| {
            a.now
                .borrow()
                .as_ref()
                .is_some_and(|n| n.release_id == obj.id())
        });
        if playing {
            l.add_css_class("playing-dot");
        } else {
            l.remove_css_class("playing-dot");
        }
    });
    // Ninguna columna se expande: con `expand` GTK reparte el espacio sobrante entre las que
    // lo tienen y al estirar una se movían otras. Cada una se queda en el ancho que le des.
    let col = gtk::ColumnViewColumn::builder()
        .title(title)
        .factory(&factory)
        .resizable(true)
        .expand(false)
        .id(id)
        .build();
    if width > 0 {
        col.set_fixed_width(width);
    }
    // El orden lo hace SQLite: el sorter solo sirve para que la cabecera sea clicable.
    col.set_sorter(Some(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal)));
    col
}

fn mark_column(app: &std::rc::Weak<App>) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            let l = gtk::Label::new(None);
            l.add_css_class("mark-dot");
            item.set_child(Some(&l));
        }
    });
    let app_w = app.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let (Some(obj), Some(l)) = (
            item.item().and_downcast::<RowObject>(),
            item.child().and_downcast::<gtk::Label>(),
        ) else {
            return;
        };
        for m in Mark::ALL {
            l.remove_css_class(&format!("mark-{}", m.key()));
        }
        l.remove_css_class("playing-dot");
        let playing = app_w.upgrade().is_some_and(|a| {
            a.now
                .borrow()
                .as_ref()
                .is_some_and(|n| n.release_id == obj.id())
        });
        if playing {
            l.set_label("♪");
            l.add_css_class("playing-dot");
            l.set_tooltip_text(Some("Sonando"));
        } else if let Some(m) = obj.mark() {
            l.set_label("●");
            l.add_css_class(&format!("mark-{}", m.key()));
            l.set_tooltip_text(Some(m.label()));
        } else {
            l.set_label("");
            l.set_tooltip_text(None);
        }
    });
    gtk::ColumnViewColumn::builder()
        .title("")
        .factory(&factory)
        .fixed_width(28)
        .build()
}

/// (id, título, ancho por defecto, texto de la celda, se puede ordenar).
type ColumnSpec = (&'static str, &'static str, i32, fn(&Row) -> String, bool);

/// Columnas de la lista en su orden por defecto.
const COLUMNS: [ColumnSpec; 8] = [
    ("artista", "Artista", 190, |r| r.artist.clone(), true),
    ("titulo", "Título", 220, |r| r.title.clone(), true),
    ("sello", "Sello", 150, |r| r.label.clone(), true),
    ("catno", "Catno", 90, |r| r.catno.clone(), true),
    ("anyo", "Año", 52, opt_year, true),
    ("pais", "País", 90, |r| r.country.clone(), true),
    ("estilos", "Estilos", 160, |r| r.styles.clone(), false),
    ("videos", "Vídeos", 62, videos_mark, false),
];

/// Menú de la cabecera (clic derecho): una casilla por columna.
fn columns_menu() -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    for (id, title, ..) in COLUMNS {
        menu.append(Some(title), Some(&format!("cols.{id}")));
    }
    menu
}

fn opt_year(r: &Row) -> String {
    r.year.map(|y| y.to_string()).unwrap_or_default()
}

fn videos_mark(r: &Row) -> String {
    if r.n_videos > 0 {
        format!("▶ {}", r.n_videos)
    } else {
        String::new()
    }
}

// ---------- Ajustes (T23) ----------

fn settings_popover(app: &Rc<App>) -> gtk::Popover {
    let grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(8)
        .css_classes(["settings-popover"])
        .build();
    let c = app.config.borrow().clone();

    let names: Vec<&str> = THEMES.iter().map(|t| t.name).collect();
    let theme_dd = gtk::DropDown::from_strings(&names);
    theme_dd.set_selected(theme_index(&c.theme) as u32);
    {
        let app = Rc::clone(app);
        theme_dd.connect_selected_notify(move |dd| {
            let Some(t) = THEMES.get(dd.selected() as usize) else {
                return;
            };
            app.css.load_from_string(&css_for(&t.effective()));
            app.config.borrow_mut().theme = t.id.into();
            app.config.borrow().save();
        });
    }

    let audio_names: Vec<&str> = AudioServer::ALL.iter().map(|a| a.label()).collect();
    let audio_dd = gtk::DropDown::from_strings(&audio_names);
    audio_dd.set_selected(
        AudioServer::ALL
            .iter()
            .position(|a| *a == c.audio)
            .unwrap_or(0) as u32,
    );
    {
        let app = Rc::clone(app);
        audio_dd.connect_selected_notify(move |dd| {
            if let Some(a) = AudioServer::ALL.get(dd.selected() as usize) {
                app.config.borrow_mut().audio = *a;
                app.config.borrow().save();
                app.restart_player();
            }
        });
    }

    let device = gtk::Entry::builder()
        .text(&c.audio_device)
        .placeholder_text("por defecto (p. ej. hw:1,0)")
        .build();
    {
        let app = Rc::clone(app);
        device.connect_activate(move |e| {
            app.config.borrow_mut().audio_device = e.text().trim().to_owned();
            app.config.borrow().save();
            app.restart_player();
        });
    }

    let video = gtk::CheckButton::with_label("Mostrar vídeo al reproducir");
    video.set_active(c.video);
    {
        let app = Rc::clone(app);
        video.connect_toggled(move |v| {
            app.config.borrow_mut().video = v.is_active();
            app.config.borrow().save();
            app.playbar.video.set_active(v.is_active());
        });
    }

    let dir_label = gtk::Label::builder()
        .label(
            c.bases_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
        )
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .max_width_chars(32)
        .build();
    let dir_btn = gtk::Button::with_label("Cambiar…");
    {
        let (app, dir_label) = (Rc::clone(app), dir_label.clone());
        dir_btn.connect_clicked(move |_| {
            let d = gtk::FileDialog::builder()
                .title("Carpeta de las bases")
                .build();
            let (app2, dir_label) = (Rc::clone(&app), dir_label.clone());
            d.select_folder(Some(&app.window), gtk::gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r
                    && let Some(p) = f.path()
                {
                    dir_label.set_label(&p.display().to_string());
                    app2.config.borrow_mut().bases_dir = p;
                    app2.config.borrow().save();
                }
            });
        });
    }
    let dir_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    dir_box.append(&dir_label);
    dir_box.append(&dir_btn);

    // Columnas visibles (las mismas casillas que el clic derecho en la cabecera).
    let cols_box = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(4)
        .min_children_per_line(4)
        .column_spacing(6)
        .build();
    for (id, title, ..) in COLUMNS {
        let c = gtk::CheckButton::with_label(title);
        c.set_action_name(Some(&format!("cols.{id}")));
        cols_box.insert(&c, -1);
    }

    let rows: [(&str, gtk::Widget); 6] = [
        ("Tema", theme_dd.upcast()),
        ("Sonido", audio_dd.upcast()),
        ("Dispositivo", device.upcast()),
        ("Vídeo", video.upcast()),
        ("Bases nuevas en", dir_box.upcast()),
        ("Columnas", cols_box.upcast()),
    ];
    for (i, (l, w)) in rows.into_iter().enumerate() {
        grid.attach(&dim(l), 0, i as i32, 1, 1);
        grid.attach(&w, 1, i as i32, 1, 1);
    }
    let help = dim(
        "Atajos: Espacio play/pausa · ←/→ ±10 s · ,/. vídeo anterior/siguiente · 1–4 marcadores \
         (0 quita) · N/P disco siguiente/anterior · D digging · V vídeo · Alt+1–9 a maleta · Ctrl+F buscar · \
         clic derecho en la cabecera para ocultar columnas",
    );
    help.set_wrap(true);
    help.set_max_width_chars(48);
    grid.attach(&help, 0, 6, 2, 1);
    gtk::Popover::builder().child(&grid).build()
}

// ---------- Bases y búsquedas guardadas (menús) ----------

fn bases_popover(app: &Rc<App>, menu: &gtk::MenuButton) -> gtk::Popover {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.add_css_class("settings-popover");
    let pop = gtk::Popover::builder().child(&b).build();
    let fill = {
        let (app, b, pop) = (Rc::downgrade(app), b.clone(), pop.clone());
        move || {
            let Some(app) = app.upgrade() else { return };
            while let Some(c) = b.first_child() {
                b.remove(&c);
            }
            let import = gtk::Button::with_label("Importar dump…");
            import.add_css_class("suggested-action");
            let open = gtk::Button::with_label("Abrir base…");
            {
                let (app, pop) = (Rc::clone(&app), pop.clone());
                import.connect_clicked(move |_| {
                    pop.popdown();
                    app.import_dialog();
                });
            }
            {
                let (app, pop) = (Rc::clone(&app), pop.clone());
                open.connect_clicked(move |_| {
                    pop.popdown();
                    app.choose_base();
                });
            }
            b.append(&import);
            b.append(&open);
            let recent = app.config.borrow().recent_bases.clone();
            if !recent.is_empty() {
                let l = dim("Recientes");
                l.set_margin_top(6);
                b.append(&l);
            }
            let current = app.base().map(|x| x.info().path.clone());
            for p in recent {
                let name = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let btn = gtk::Button::with_label(&if Some(&p) == current.as_ref() {
                    format!("● {name}")
                } else {
                    name
                });
                btn.add_css_class("flat");
                btn.set_tooltip_text(Some(&p.display().to_string()));
                if let Some(child) = btn.child().and_downcast::<gtk::Label>() {
                    child.set_xalign(0.0);
                }
                let (app, pop) = (Rc::clone(&app), pop.clone());
                btn.connect_clicked(move |_| {
                    pop.popdown();
                    app.open_base(&p);
                });
                b.append(&btn);
            }
        }
    };
    menu.connect_active_notify(move |m| {
        if m.is_active() {
            fill();
        }
    });
    pop
}

/// Función que vuelve a pintar un menú.
type Refill = Rc<dyn Fn()>;

fn searches_popover(app: &Rc<App>, menu: &gtk::MenuButton) -> gtk::Popover {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.add_css_class("settings-popover");
    let pop = gtk::Popover::builder().child(&b).build();
    let fill: Rc<RefCell<Option<Refill>>> = Rc::default();
    let f = {
        let (app, b, pop, fill_ref) =
            (Rc::downgrade(app), b.clone(), pop.clone(), Rc::clone(&fill));
        move || {
            let Some(app) = app.upgrade() else { return };
            while let Some(c) = b.first_child() {
                b.remove(&c);
            }
            let Some(base) = app.base() else {
                b.append(&dim("Abre una base primero."));
                return;
            };
            let save = gtk::Button::with_label("Guardar la búsqueda actual…");
            {
                let (app, pop) = (Rc::clone(&app), pop.clone());
                save.connect_clicked(move |_| {
                    pop.popdown();
                    let a = Rc::clone(&app);
                    dialogs::ask_text(&app.window, "Guardar búsqueda", "", "Guardar", move |n| {
                        let base = a.base().ok_or("No hay base abierta")?;
                        let (f, so) = (a.filters().get(), a.sort.get());
                        let id = base
                            .user()
                            .save_search(&n, &f, so)
                            .map_err(|e| e.to_string())?;
                        a.active_search.replace(Some((id, n.clone(), f, so)));
                        a.set_status(&format!("Búsqueda «{n}» guardada"));
                        a.save_session();
                        Ok(())
                    });
                });
            }
            b.append(&save);
            // Con una búsqueda cargada y cambiada: guardar los cambios con el mismo nombre.
            let active = app.active_search.borrow().clone();
            if let Some((_, name, filtros, sort)) = active
                && (app.filters().get() != filtros || app.sort.get() != sort)
            {
                let upd = gtk::Button::with_label(&format!("Guardar cambios en «{name}»"));
                let (app, pop) = (Rc::clone(&app), pop.clone());
                upd.connect_clicked(move |_| {
                    pop.popdown();
                    let Some(base) = app.base() else { return };
                    let (f, so) = (app.filters().get(), app.sort.get());
                    match base.user().save_search(&name, &f, so) {
                        Ok(id) => {
                            app.active_search.replace(Some((id, name.clone(), f, so)));
                            app.set_status(&format!("Búsqueda «{name}» actualizada"));
                        }
                        Err(e) => app.set_status(&e.to_string()),
                    }
                });
                b.append(&upd);
            }
            let saved = base.user().saved_searches().unwrap_or_default();
            if saved.is_empty() {
                b.append(&dim("No hay búsquedas guardadas."));
            }
            for s in saved {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                let load = gtk::Button::with_label(&s.name);
                load.add_css_class("flat");
                load.set_hexpand(true);
                if let Some(child) = load.child().and_downcast::<gtk::Label>() {
                    child.set_xalign(0.0);
                }
                let rename = gtk::Button::from_icon_name("document-edit-symbolic");
                rename.set_tooltip_text(Some("Renombrar"));
                rename.add_css_class("flat");
                let delete = gtk::Button::from_icon_name("user-trash-symbolic");
                delete.set_tooltip_text(Some("Borrar"));
                delete.add_css_class("flat");
                {
                    let (app, pop, filtros, sort) =
                        (Rc::clone(&app), pop.clone(), s.filtros.clone(), s.sort);
                    let (id, name) = (s.id, s.name.clone());
                    load.connect_clicked(move |_| {
                        pop.popdown();
                        app.active_search
                            .replace(Some((id, name.clone(), filtros.clone(), sort)));
                        app.filters().set(&filtros);
                        app.apply_sort(sort);
                        app.run_search();
                    });
                }
                {
                    let (app, pop, name, id) = (Rc::clone(&app), pop.clone(), s.name.clone(), s.id);
                    rename.connect_clicked(move |_| {
                        pop.popdown();
                        let a = Rc::clone(&app);
                        dialogs::ask_text(
                            &app.window,
                            "Renombrar búsqueda",
                            &name,
                            "Renombrar",
                            move |n| {
                                let base = a.base().ok_or("No hay base abierta")?;
                                base.user().rename_search(id, &n).map_err(|e| e.to_string())
                            },
                        );
                    });
                }
                {
                    let (app, id, fill_ref) = (Rc::clone(&app), s.id, Rc::clone(&fill_ref));
                    delete.connect_clicked(move |_| {
                        if let Some(base) = app.base() {
                            let _ = base.user().delete_search(id);
                        }
                        // Se vuelve a pintar la lista después de este manejador.
                        let fill_ref = Rc::clone(&fill_ref);
                        glib::idle_add_local_once(move || {
                            if let Some(f) = fill_ref.borrow().as_ref() {
                                f();
                            }
                        });
                    });
                }
                row.append(&load);
                row.append(&rename);
                row.append(&delete);
                b.append(&row);
            }
        }
    };
    let f: Refill = Rc::new(f);
    fill.replace(Some(Rc::clone(&f)));
    menu.connect_active_notify(move |m| {
        if m.is_active() {
            f();
        }
    });
    pop
}

// ---------- Construcción ----------

/// Construye y muestra la ventana.
pub fn build(gtk_app: &gtk::Application) {
    let config = Config::load();
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let css = gtk::CssProvider::new();
    css.load_from_string(&css_for(&Theme::by_id(&config.theme).effective()));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let header = gtk::HeaderBar::new();
    let title_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    title_box.set_valign(gtk::Align::Center);
    let base_title = gtk::Label::builder()
        .label("Vault")
        .css_classes(["base-title"])
        .build();
    let base_subtitle = gtk::Label::builder()
        .css_classes(["dim-label", "base-subtitle"])
        .visible(false)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title_box.append(&base_title);
    title_box.append(&base_subtitle);
    header.set_title_widget(Some(&title_box));

    let model = ReleaseModel::default();
    let selection = gtk::SingleSelection::builder()
        .model(&model)
        .autoselect(false)
        .can_unselect(true)
        .build();
    let view = gtk::ColumnView::builder()
        .model(&selection)
        .show_column_separators(true)
        .reorderable(true)
        .build();
    let playbar = PlayBar::new(config.volume, config.video);

    let window = gtk::ApplicationWindow::builder()
        .application(gtk_app)
        .title("Vault")
        .default_width(1400)
        .default_height(860)
        .titlebar(&header)
        .build();

    let center = gtk::Stack::new();
    let user_path = userdb::default_path();
    let app = Rc::new_cyclic(|weak: &std::rc::Weak<App>| {
        // Paneles.
        let filters = {
            let weak = weak.clone();
            FilterPanel::new(move || {
                if let Some(a) = weak.upgrade() {
                    a.schedule_search();
                }
            })
        };
        let detail = {
            let w1 = weak.clone();
            let w2 = weak.clone();
            let w3 = weak.clone();
            let w4 = weak.clone();
            let w5 = weak.clone();
            DetailPanel::new(DetailActions {
                play_video: Box::new(move |i| {
                    if let Some(a) = w1.upgrade()
                        && let Some(id) = a.detail().current()
                    {
                        let same = a
                            .now
                            .borrow()
                            .as_ref()
                            .is_some_and(|n| n.release_id == id && n.index == i);
                        if same {
                            a.toggle_pause();
                        } else {
                            a.play(id, i);
                        }
                    }
                }),
                set_mark: Box::new(move |m| {
                    if let Some(a) = w2.upgrade()
                        && let Some(id) = a.detail().current()
                    {
                        // Tras la señal: el panel se vuelve a pintar entero.
                        let a2 = Rc::clone(&a);
                        glib::idle_add_local_once(move || a2.set_mark(id, m));
                    }
                }),
                set_suitcase: Box::new(move |sid, on| {
                    if let Some(a) = w3.upgrade()
                        && let Some(base) = a.base()
                        && let Some(id) = a.detail().current()
                    {
                        let r = if on {
                            base.user().add_to_suitcase(sid, id)
                        } else {
                            base.user().remove_from_suitcase(sid, id)
                        };
                        if let Err(e) = r {
                            a.set_status(&e.to_string());
                        }
                        let a2 = Rc::clone(&a);
                        glib::idle_add_local_once(move || a2.reload_suitcases());
                    }
                }),
                save_note: Box::new(move |id, text| {
                    if let Some(a) = w4.upgrade()
                        && let Some(base) = a.base()
                        && let Err(e) = base.user().set_note(id, &text)
                    {
                        a.set_status(&e.to_string());
                    }
                }),
                search: Box::new(move |f| {
                    if let Some(a) = w5.upgrade() {
                        // Tras la señal: el panel se vuelve a pintar con el resultado.
                        let a2 = Rc::clone(&a);
                        glib::idle_add_local_once(move || {
                            a2.filters().set(&f);
                            a2.run_search();
                        });
                    }
                }),
            })
        };
        App {
            window: window.clone(),
            config: RefCell::new(config),
            css,
            user_path,
            base: RefCell::default(),
            model,
            selection,
            view,
            status: gtk::Label::builder()
                .xalign(0.0)
                .css_classes(["dim-label", "statusbar"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
            base_title,
            base_subtitle,
            filters,
            detail,
            playbar,
            suitcase_list: gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .build(),
            search: RefCell::default(),
            generation: Cell::new(0),
            debounce: RefCell::default(),
            sort: Cell::new(Sort::default()),
            player: RefCell::default(),
            player_events: RefCell::default(),
            now: RefCell::default(),
            center,
            warming: RefCell::default(),
            active_search: RefCell::default(),
            pending_select: Cell::new(None),
            session_timer: RefCell::default(),
        }
    });

    // Columnas: en el orden, ancho y visibilidad guardados.
    let weak = Rc::downgrade(&app);
    let header_menu = columns_menu();
    let mark_col = mark_column(&weak);
    mark_col.set_header_menu(Some(&header_menu));
    app.view.append_column(&mark_col);
    let defaults: Vec<(&str, i32)> = COLUMNS.iter().map(|c| (c.0, c.2)).collect();
    let state = vault::config::ordered_columns(&app.config.borrow().columns, &defaults);
    let actions = gtk::gio::SimpleActionGroup::new();
    for st in &state {
        let Some((id, title, _, get, sortable)) = COLUMNS.iter().find(|c| c.0 == st.id).copied()
        else {
            continue;
        };
        let c = text_column(&weak, id, title, st.width, get);
        if !sortable {
            c.set_sorter(None::<&gtk::Sorter>);
        }
        c.set_visible(st.visible);
        c.set_header_menu(Some(&header_menu));
        app.view.append_column(&c);

        let action = gtk::gio::SimpleAction::new_stateful(id, None, &st.visible.to_variant());
        let (col, a) = (c.clone(), Rc::downgrade(&app));
        action.connect_change_state(move |act, value| {
            let Some(on) = value.and_then(|v| v.get::<bool>()) else {
                return;
            };
            act.set_state(&on.to_variant());
            col.set_visible(on);
            if let Some(a) = a.upgrade() {
                a.save_columns();
            }
        });
        actions.add_action(&action);
    }
    window.insert_action_group("cols", Some(&actions));
    // Guardar anchos y orden medio segundo después de estirar o mover una columna (no en cada
    // píxel del arrastre).
    {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let schedule = {
            let (a, pending) = (Rc::downgrade(&app), Rc::clone(&pending));
            move || {
                if let Some(t) = pending.take() {
                    t.remove();
                }
                let (a, p2) = (a.clone(), Rc::clone(&pending));
                let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    p2.replace(None);
                    if let Some(a) = a.upgrade() {
                        a.save_columns();
                    }
                });
                pending.replace(Some(id));
            }
        };
        let cols = app.view.columns();
        for i in 0..cols.n_items() {
            if let Some(c) = cols.item(i).and_downcast::<gtk::ColumnViewColumn>() {
                let f = schedule.clone();
                c.connect_fixed_width_notify(move |_| f());
            }
        }
        cols.connect_items_changed(move |_, _, _, _| schedule());
    }
    if let Some(sorter) = app.view.sorter() {
        let weak = Rc::downgrade(&app);
        sorter.connect_changed(move |_, _| {
            if let Some(a) = weak.upgrade() {
                a.on_sort_changed();
            }
        });
    }
    {
        let weak = Rc::downgrade(&app);
        app.selection.connect_selected_notify(move |_| {
            if let Some(a) = weak.upgrade() {
                a.show_detail();
            }
        });
    }
    {
        let weak = Rc::downgrade(&app);
        app.view.connect_activate(move |_, pos| {
            if let Some(a) = weak.upgrade() {
                a.selection.set_selected(pos);
                a.play_selected();
            }
        });
    }

    // Centro: lista o bienvenida.
    let list_scroll = gtk::ScrolledWindow::builder()
        .child(&app.view)
        .vexpand(true)
        .hexpand(true)
        .build();
    let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_box.append(&list_scroll);
    list_box.append(&app.status);
    let welcome = gtk::Box::new(gtk::Orientation::Vertical, 12);
    welcome.set_valign(gtk::Align::Center);
    welcome.set_halign(gtk::Align::Center);
    welcome.append(
        &gtk::Label::builder()
            .label("Vault")
            .css_classes(["placeholder-title"])
            .build(),
    );
    let welcome_text = dim(
        "Importa un dump de Discogs (discogs_AAAAMMDD_releases.xml.gz) o abre una base ya importada.",
    );
    // Que el texto se parta: si no, su ancho limita hasta dónde se puede mover el separador.
    welcome_text.set_wrap(true);
    welcome_text.set_max_width_chars(40);
    welcome_text.set_justify(gtk::Justification::Center);
    welcome.append(&welcome_text);
    let wb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    wb.set_halign(gtk::Align::Center);
    let w_import = gtk::Button::with_label("Importar dump…");
    w_import.add_css_class("suggested-action");
    let w_open = gtk::Button::with_label("Abrir base…");
    wb.append(&w_import);
    wb.append(&w_open);
    welcome.append(&wb);
    {
        let a = Rc::clone(&app);
        w_import.connect_clicked(move |_| a.import_dialog());
    }
    {
        let a = Rc::clone(&app);
        w_open.connect_clicked(move |_| a.choose_base());
    }
    app.center.add_named(&welcome, Some("welcome"));
    app.center.add_named(&list_box, Some("list"));
    app.center.set_visible_child_name("welcome");

    // Izquierda: filtros + maletas.
    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    app.filters.root.set_vexpand(true);
    left.append(&app.filters.root);
    left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let suit_head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    suit_head.set_margin_start(10);
    suit_head.set_margin_end(6);
    suit_head.set_margin_top(4);
    let st = dim("Maletas");
    st.add_css_class("section-title");
    st.set_hexpand(true);
    suit_head.append(&st);
    let add_suit = gtk::Button::from_icon_name("list-add-symbolic");
    add_suit.add_css_class("flat");
    add_suit.set_tooltip_text(Some("Nueva maleta"));
    {
        let a = Rc::clone(&app);
        add_suit.connect_clicked(move |_| a.new_suitcase());
    }
    suit_head.append(&add_suit);
    left.append(&suit_head);
    let suit_scroll = gtk::ScrolledWindow::builder()
        .child(&app.suitcase_list)
        .min_content_height(60)
        .max_content_height(220)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    left.append(&suit_scroll);
    {
        let a = Rc::downgrade(&app);
        app.suitcase_list.connect_row_activated(move |_, row| {
            if let Some(a) = a.upgrade()
                && let Ok(id) = row.widget_name().parse::<i64>()
            {
                a.filters().select_suitcase(Some(id));
            }
        });
    }
    app.suitcase_list.set_activate_on_single_click(true);

    // Lista | detalle, con separador movible. Al cambiar el tamaño de la ventana el detalle
    // conserva su ancho y la lista se queda con el resto; el ancho se recuerda.
    app.detail.root.set_width_request(300);
    let inner = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&app.center)
        .end_child(&app.detail.root)
        .resize_start_child(true)
        .resize_end_child(false)
        // La lista puede estrecharse (tiene barra horizontal); el detalle no baja de 300 px.
        .shrink_start_child(true)
        .shrink_end_child(false)
        .wide_handle(true)
        .build();
    {
        // La posición inicial solo se puede poner cuando el panel ya tiene ancho.
        let ready = Rc::new(Cell::new(false));
        let (a, r) = (Rc::downgrade(&app), Rc::clone(&ready));
        inner.connect_map(move |p| {
            let (a, r, p) = (a.clone(), Rc::clone(&r), p.clone());
            glib::idle_add_local_once(move || {
                if let Some(a) = a.upgrade() {
                    let w = a.config.borrow().detail_width.max(300);
                    if p.width() > w + 400 {
                        p.set_position(p.width() - w);
                    }
                }
                r.set(true);
            });
        });
        let a = Rc::downgrade(&app);
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        inner.connect_position_notify(move |p| {
            if !ready.get() {
                return;
            }
            let Some(a) = a.upgrade() else { return };
            let w = p.width() - p.position();
            if w < 300 || a.config.borrow().detail_width == w {
                return;
            }
            a.config.borrow_mut().detail_width = w;
            // Guardar medio segundo después de soltar, no en cada píxel.
            if let Some(t) = pending.take() {
                t.remove();
            }
            let (a2, p2) = (Rc::downgrade(&a), Rc::clone(&pending));
            let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                p2.replace(None);
                if let Some(a) = a2.upgrade() {
                    a.config.borrow().save();
                }
            });
            pending.replace(Some(id));
        });
    }
    let outer = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .start_child(&left)
        .end_child(&inner)
        .resize_start_child(false)
        .shrink_start_child(false)
        .vexpand(true)
        .build();
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.append(&outer);
    main.append(&app.playbar.root);
    window.set_child(Some(&main));

    // Cabecera: bases, búsquedas guardadas, ajustes.
    let bases_btn = gtk::MenuButton::builder()
        .label("Bases")
        .tooltip_text("Importar, abrir y cambiar de base")
        .build();
    bases_btn.set_popover(Some(&bases_popover(&app, &bases_btn)));
    header.pack_start(&bases_btn);
    let searches_btn = gtk::MenuButton::builder()
        .label("Búsquedas")
        .tooltip_text("Búsquedas guardadas")
        .build();
    searches_btn.set_popover(Some(&searches_popover(&app, &searches_btn)));
    header.pack_start(&searches_btn);
    let settings_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Ajustes")
        .popover(&settings_popover(&app))
        .build();
    header.pack_end(&settings_btn);

    // Barra de reproducción.
    {
        let a = Rc::clone(&app);
        app.playbar.play.connect_clicked(move |_| a.toggle_pause());
    }
    {
        let a = Rc::clone(&app);
        app.playbar.back.connect_clicked(move |_| a.seek(-10.0));
    }
    {
        let a = Rc::clone(&app);
        app.playbar.fwd.connect_clicked(move |_| a.seek(10.0));
    }
    {
        let a = Rc::clone(&app);
        app.playbar.prev.connect_clicked(move |_| {
            a.step_video(-1);
        });
    }
    {
        let a = Rc::clone(&app);
        app.playbar.next.connect_clicked(move |_| {
            a.step_video(1);
        });
    }
    {
        let a = Rc::downgrade(&app);
        app.playbar.scale.connect_change_value(move |_, _, v| {
            if let Some(a) = a.upgrade()
                && let Some(p) = a.player.borrow().as_ref()
            {
                let _ = p.seek_absolute(v);
            }
            glib::Propagation::Proceed
        });
    }
    {
        let a = Rc::downgrade(&app);
        app.playbar.volume.connect_value_changed(move |s| {
            if let Some(a) = a.upgrade() {
                if let Some(p) = a.player.borrow().as_ref() {
                    let _ = p.set_volume(s.value());
                }
                a.config.borrow_mut().volume = s.value();
            }
        });
    }
    {
        let a = Rc::downgrade(&app);
        app.playbar.video.connect_toggled(move |t| {
            if let Some(a) = a.upgrade()
                && let Some(p) = a.player.borrow().as_ref()
            {
                let _ = p.set_video(t.is_active());
            }
        });
    }
    {
        let a = Rc::downgrade(&app);
        app.playbar.digging.connect_toggled(move |t| {
            if let Some(a) = a.upgrade()
                && t.is_active()
            {
                a.set_status(
                    "Digging: 1–4 marcan y pasan al siguiente · N/P disco · Espacio pausa",
                );
                a.view.grab_focus();
                if a.now.borrow().is_none() {
                    if a.selection.selected() == gtk::INVALID_LIST_POSITION {
                        a.step_release(0);
                    }
                    a.play_selected();
                }
            }
        });
    }

    // Teclado.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let a = Rc::downgrade(&app);
        keys.connect_key_pressed(move |_, key, _, state| match a.upgrade() {
            Some(a) => a.on_key(key, state),
            None => glib::Propagation::Proceed,
        });
    }
    window.add_controller(keys);

    // Hilos: búsqueda y eventos de mpv.
    let (tx, rx) = async_channel::unbounded::<Response>();
    match SearchWorker::spawn(tx) {
        Ok(w) => {
            app.search.replace(Some(w));
        }
        Err(e) => tracing::error!("no se pudo lanzar el hilo de búsqueda: {e}"),
    }
    {
        let a = Rc::downgrade(&app);
        glib::spawn_future_local(async move {
            while let Ok(r) = rx.recv().await {
                match a.upgrade() {
                    Some(a) => a.on_results(r),
                    None => break,
                }
            }
        });
    }
    {
        let a = Rc::downgrade(&app);
        glib::timeout_add_local(Duration::from_millis(100), move || match a.upgrade() {
            Some(a) => {
                a.poll_player();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }
    {
        let a = Rc::clone(&app);
        window.connect_close_request(move |_| {
            a.save_session();
            a.save_columns();
            a.config.borrow().save();
            a.player.replace(None);
            glib::Propagation::Proceed
        });
    }

    window.present();

    // Reabrir la última base (T08).
    let last = app.config.borrow().recent_bases.first().cloned();
    if let Some(p) = last {
        if p.is_file() {
            app.open_base(&p);
        } else {
            app.config.borrow_mut().forget_recent(&p);
        }
    }
    // Ruta de una base pasada por la línea de órdenes: `vault base.db`.
    if let Some(p) = std::env::args().nth(1).map(PathBuf::from)
        && p.is_file()
    {
        app.open_base(&p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indice_de_tema() {
        assert_eq!(theme_index("dracula"), 0);
        assert_eq!(THEMES[theme_index("nord")].id, "nord");
        assert_eq!(theme_index("no-existe"), 0);
    }

    #[test]
    fn css_incluye_tema_y_estilo_propio() {
        let css = css_for(&THEMES[0].effective());
        assert!(css.contains("@define-color at_fondo #282a36;"));
        assert!(css.contains(".mark-interesa { color: @at_ok; }"));
    }
}
