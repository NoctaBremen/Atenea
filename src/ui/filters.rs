//! Panel lateral de filtros (T11): texto, artista, sello, años, país, géneros, estilos,
//! formato, solo con vídeos, marcadores y maleta. Cualquier cambio llama a `on_change` (la
//! ventana espera un poco antes de buscar, para no lanzar una consulta por tecla).

use std::cell::RefCell;
use std::rc::Rc;

use atenea::db::base::FORMAT_DESCRIPTIONS;
use atenea::db::{Base, Filtros, Mark};
use atenea::userdb::Suitcase;
use gtk::prelude::*;

/// Lista de casillas con buscador (géneros, estilos).
struct CheckList {
    root: gtk::Box,
    search: gtk::SearchEntry,
    list: gtk::Box,
    checks: RefCell<Vec<(String, gtk::CheckButton)>>,
}

impl CheckList {
    fn new(title: &str, with_search: bool) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.append(&section_label(title));
        let search = gtk::SearchEntry::builder()
            .placeholder_text(format!("Buscar {}", title.to_lowercase()))
            .build();
        if with_search {
            root.append(&search);
        }
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .min_content_height(80)
            .max_content_height(180)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        root.append(&scroll);
        let this = Rc::new(Self {
            root,
            search,
            list,
            checks: RefCell::default(),
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_search_changed(move |e| {
            if let Some(this) = weak.upgrade() {
                let q = e.text().to_lowercase();
                for (name, c) in this.checks.borrow().iter() {
                    c.set_visible(q.is_empty() || name.to_lowercase().contains(&q));
                }
            }
        });
        this
    }

    /// Rellena con (nombre, nº de discos); conserva lo marcado si sigue existiendo.
    fn fill(&self, items: &[(String, i64)], on_change: &Rc<dyn Fn()>) {
        let keep = self.selected();
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut checks = Vec::new();
        for (name, n) in items {
            let c = gtk::CheckButton::with_label(&format!("{name}  ({n})"));
            c.set_active(keep.contains(name));
            let cb = Rc::clone(on_change);
            c.connect_toggled(move |_| cb());
            self.list.append(&c);
            checks.push((name.clone(), c));
        }
        self.checks.replace(checks);
    }

    fn selected(&self) -> Vec<String> {
        self.checks
            .borrow()
            .iter()
            .filter(|(_, c)| c.is_active())
            .map(|(n, _)| n.clone())
            .collect()
    }

    fn set_selected(&self, names: &[String]) {
        for (n, c) in self.checks.borrow().iter() {
            c.set_active(names.contains(n));
        }
    }
}

fn section_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["dim-label", "section-title"])
        .margin_top(8)
        .build()
}

/// El panel.
pub struct FilterPanel {
    /// Widget raíz (dentro de un `ScrolledWindow`).
    pub root: gtk::ScrolledWindow,
    /// Texto libre (la ventana le pone el foco con Ctrl+F).
    pub texto: gtk::SearchEntry,
    artista: gtk::Entry,
    sello: gtk::Entry,
    desde: gtk::SpinButton,
    hasta: gtk::SpinButton,
    pais: gtk::DropDown,
    paises: RefCell<Vec<String>>,
    generos: Rc<CheckList>,
    estilos: Rc<CheckList>,
    formatos: Vec<(String, gtk::ToggleButton)>,
    videos: gtk::CheckButton,
    marcas: Vec<(Mark, gtk::ToggleButton)>,
    ocultar_no: gtk::CheckButton,
    maleta: gtk::DropDown,
    maletas: RefCell<Vec<i64>>,
    on_change: Rc<dyn Fn()>,
    /// Mientras se rellenan los widgets desde código no se avisa de cambios (lo comparte el
    /// cierre de `on_change`).
    quiet: Rc<std::cell::Cell<bool>>,
}

impl FilterPanel {
    /// Construye el panel; `on_change` se llama en cada cambio hecho por el usuario.
    pub fn new(on_change: impl Fn() + 'static) -> Rc<Self> {
        let quiet = Rc::new(std::cell::Cell::new(false));
        let quiet_cb = Rc::clone(&quiet);
        let on_change: Rc<dyn Fn()> = Rc::new(move || {
            if !quiet_cb.get() {
                on_change();
            }
        });

        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.add_css_class("filter-panel");
        b.set_margin_start(10);
        b.set_margin_end(10);
        b.set_margin_top(8);
        b.set_margin_bottom(8);

        let texto = gtk::SearchEntry::builder()
            .placeholder_text("Buscar (\"frase exacta\")")
            .build();
        b.append(&texto);
        let artista = gtk::Entry::builder().placeholder_text("Artista").build();
        let sello = gtk::Entry::builder().placeholder_text("Sello").build();
        b.append(&artista);
        b.append(&sello);

        b.append(&section_label("Años"));
        let years = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        // 0 = sin límite.
        let desde = gtk::SpinButton::with_range(0.0, 2100.0, 1.0);
        let hasta = gtk::SpinButton::with_range(0.0, 2100.0, 1.0);
        for s in [&desde, &hasta] {
            s.set_hexpand(true);
            s.set_tooltip_text(Some("0 = sin límite"));
        }
        years.append(&desde);
        years.append(&gtk::Label::new(Some("–")));
        years.append(&hasta);
        b.append(&years);

        b.append(&section_label("País"));
        let pais = gtk::DropDown::from_strings(&["Todos"]);
        pais.set_enable_search(true);
        b.append(&pais);

        let generos = CheckList::new("Géneros", false);
        let estilos = CheckList::new("Estilos", true);
        b.append(&generos.root);
        b.append(&estilos.root);

        b.append(&section_label("Formato"));
        let fbox = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(4)
            .row_spacing(4)
            .column_spacing(4)
            .build();
        let formatos: Vec<(String, gtk::ToggleButton)> = FORMAT_DESCRIPTIONS
            .iter()
            .map(|d| {
                let t = gtk::ToggleButton::with_label(d);
                fbox.insert(&t, -1);
                ((*d).to_owned(), t)
            })
            .collect();
        b.append(&fbox);

        let videos = gtk::CheckButton::with_label("Solo con vídeos");
        b.append(&videos);

        b.append(&section_label("Marcadores"));
        let mbox = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(2)
            .row_spacing(4)
            .column_spacing(4)
            .build();
        let marcas: Vec<(Mark, gtk::ToggleButton)> = Mark::ALL
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let t = gtk::ToggleButton::with_label(m.label());
                t.set_tooltip_text(Some(&format!("Tecla {}", i + 1)));
                t.add_css_class(&format!("mark-{}", m.key()));
                mbox.insert(&t, -1);
                (*m, t)
            })
            .collect();
        b.append(&mbox);
        let ocultar_no = gtk::CheckButton::with_label("Ocultar «No me interesa»");
        b.append(&ocultar_no);

        b.append(&section_label("Maleta"));
        let maleta = gtk::DropDown::from_strings(&["Todas"]);
        b.append(&maleta);

        let clear = gtk::Button::with_label("Quitar filtros");
        clear.set_margin_top(10);
        b.append(&clear);

        let root = gtk::ScrolledWindow::builder()
            .child(&b)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(260)
            .build();

        let this = Rc::new(Self {
            root,
            texto,
            artista,
            sello,
            desde,
            hasta,
            pais,
            paises: RefCell::default(),
            generos,
            estilos,
            formatos,
            videos,
            marcas,
            ocultar_no,
            maleta,
            maletas: RefCell::default(),
            on_change: Rc::clone(&on_change),
            quiet: Rc::clone(&quiet),
        });
        {
            let cb = Rc::clone(&on_change);
            this.texto.connect_search_changed(move |_| cb());
        }
        for e in [&this.artista, &this.sello] {
            let cb = Rc::clone(&on_change);
            e.connect_changed(move |_| cb());
        }
        for s in [&this.desde, &this.hasta] {
            let cb = Rc::clone(&on_change);
            s.connect_value_changed(move |_| cb());
        }
        for d in [&this.pais, &this.maleta] {
            let cb = Rc::clone(&on_change);
            d.connect_selected_notify(move |_| cb());
        }
        for (_, t) in &this.formatos {
            let cb = Rc::clone(&on_change);
            t.connect_toggled(move |_| cb());
        }
        for (_, t) in &this.marcas {
            let cb = Rc::clone(&on_change);
            t.connect_toggled(move |_| cb());
        }
        for c in [&this.videos, &this.ocultar_no] {
            let cb = Rc::clone(&on_change);
            c.connect_toggled(move |_| cb());
        }
        {
            let weak = Rc::downgrade(&this);
            clear.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.set(&Filtros::default());
                    (this.on_change)();
                }
            });
        }
        this
    }

    fn set_quiet(&self, q: bool) {
        self.quiet.set(q);
    }

    /// Rellena las listas (países, géneros, estilos) con los de la base.
    pub fn load_base(&self, base: &Base) {
        self.set_quiet(true);
        let countries = base.countries().unwrap_or_default();
        let mut names = vec!["Todos".to_owned()];
        names.extend(countries.iter().map(|(c, n)| format!("{c}  ({n})")));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.pais.set_model(Some(&gtk::StringList::new(&refs)));
        self.pais.set_selected(0);
        self.paises
            .replace(countries.into_iter().map(|(c, _)| c).collect());
        self.generos
            .fill(&base.genres().unwrap_or_default(), &self.on_change);
        self.estilos
            .fill(&base.styles().unwrap_or_default(), &self.on_change);
        self.set_quiet(false);
    }

    /// Rellena el desplegable de maletas, conservando la elegida si sigue existiendo.
    pub fn load_suitcases(&self, suitcases: &[Suitcase]) {
        let current = self.selected_suitcase();
        self.set_quiet(true);
        let mut names = vec!["Todas".to_owned()];
        names.extend(
            suitcases
                .iter()
                .map(|s| format!("{}  ({})", s.name, s.count)),
        );
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.maleta.set_model(Some(&gtk::StringList::new(&refs)));
        let ids: Vec<i64> = suitcases.iter().map(|s| s.id).collect();
        let pos = current
            .and_then(|c| ids.iter().position(|&i| i == c))
            .map_or(0, |p| p as u32 + 1);
        self.maleta.set_selected(pos);
        self.maletas.replace(ids);
        self.set_quiet(false);
    }

    fn selected_suitcase(&self) -> Option<i64> {
        let i = self.maleta.selected() as usize;
        if i == 0 {
            None
        } else {
            self.maletas.borrow().get(i - 1).copied()
        }
    }

    /// Elige una maleta (o ninguna) y avisa del cambio.
    pub fn select_suitcase(&self, id: Option<i64>) {
        let pos = id
            .and_then(|c| self.maletas.borrow().iter().position(|&i| i == c))
            .map_or(0, |p| p as u32 + 1);
        self.maleta.set_selected(pos);
    }

    /// Filtros actuales.
    pub fn get(&self) -> Filtros {
        let year = |s: &gtk::SpinButton| {
            let v = s.value_as_int();
            (v > 0).then_some(v as u16)
        };
        let pais_i = self.pais.selected() as usize;
        Filtros {
            texto: self.texto.text().trim().to_owned(),
            artista: self.artista.text().trim().to_owned(),
            sello: self.sello.text().trim().to_owned(),
            anyo_desde: year(&self.desde),
            anyo_hasta: year(&self.hasta),
            pais: if pais_i == 0 {
                String::new()
            } else {
                self.paises
                    .borrow()
                    .get(pais_i - 1)
                    .cloned()
                    .unwrap_or_default()
            },
            generos: self.generos.selected(),
            estilos: self.estilos.selected(),
            formatos: self
                .formatos
                .iter()
                .filter(|(_, t)| t.is_active())
                .map(|(d, _)| d.clone())
                .collect(),
            solo_con_videos: self.videos.is_active(),
            marcadores: self
                .marcas
                .iter()
                .filter(|(_, t)| t.is_active())
                .map(|(m, _)| *m)
                .collect(),
            ocultar_no_interesa: self.ocultar_no.is_active(),
            maleta: self.selected_suitcase(),
        }
    }

    /// Pone estos filtros en los widgets (búsqueda guardada) sin lanzar una búsqueda por cada
    /// widget; quien llama decide cuándo buscar.
    pub fn set(&self, f: &Filtros) {
        self.set_quiet(true);
        self.texto.set_text(&f.texto);
        self.artista.set_text(&f.artista);
        self.sello.set_text(&f.sello);
        self.desde.set_value(f.anyo_desde.map_or(0.0, f64::from));
        self.hasta.set_value(f.anyo_hasta.map_or(0.0, f64::from));
        let pi = self
            .paises
            .borrow()
            .iter()
            .position(|p| *p == f.pais)
            .map_or(0, |p| p as u32 + 1);
        self.pais.set_selected(pi);
        self.generos.set_selected(&f.generos);
        self.estilos.set_selected(&f.estilos);
        for (d, t) in &self.formatos {
            t.set_active(f.formatos.contains(d));
        }
        self.videos.set_active(f.solo_con_videos);
        for (m, t) in &self.marcas {
            t.set_active(f.marcadores.contains(m));
        }
        self.ocultar_no.set_active(f.ocultar_no_interesa);
        self.select_suitcase(f.maleta);
        self.set_quiet(false);
    }
}
