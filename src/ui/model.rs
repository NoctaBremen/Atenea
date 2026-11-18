//! Modelo perezoso de la lista (T10): solo guarda los ids del resultado y crea las filas al
//! pedirlas la `ColumnView`, por páginas de [`PAGE`] con una caché pequeña. Así un resultado de
//! millones de discos ocupa lo que ocupan sus ids.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use atenea::db::{Base, Mark, Row};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Filas por página.
pub const PAGE: u32 = 200;
/// Páginas que se guardan antes de vaciar la caché.
const MAX_PAGES: usize = 40;

mod row_imp {
    use super::*;

    #[derive(Default)]
    pub struct RowObject {
        pub row: RefCell<Row>,
        pub mark: Cell<Option<Mark>>,
        pub playing: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowObject {
        const NAME: &'static str = "AteneaRowObject";
        type Type = super::RowObject;
    }

    impl ObjectImpl for RowObject {}
}

glib::wrapper! {
    /// Un disco de la lista.
    pub struct RowObject(ObjectSubclass<row_imp::RowObject>);
}

impl RowObject {
    fn new(row: Row, mark: Option<Mark>) -> Self {
        let o: Self = glib::Object::new();
        o.imp().row.replace(row);
        o.imp().mark.set(mark);
        o
    }

    /// Datos de la fila.
    pub fn row(&self) -> std::cell::Ref<'_, Row> {
        self.imp().row.borrow()
    }

    /// Id de Discogs.
    pub fn id(&self) -> i64 {
        self.imp().row.borrow().id
    }

    /// Marcador.
    pub fn mark(&self) -> Option<Mark> {
        self.imp().mark.get()
    }
}

mod model_imp {
    use super::*;

    #[derive(Default)]
    pub struct ReleaseModel {
        pub ids: RefCell<Vec<i64>>,
        pub base: RefCell<Option<Rc<Base>>>,
        pub cache: RefCell<HashMap<u32, Vec<RowObject>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReleaseModel {
        const NAME: &'static str = "AteneaReleaseModel";
        type Type = super::ReleaseModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ReleaseModel {}

    impl ListModelImpl for ReleaseModel {
        fn item_type(&self) -> glib::Type {
            RowObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.ids.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            if position >= self.n_items() {
                return None;
            }
            let page = position / PAGE;
            let idx = (position % PAGE) as usize;
            if let Some(p) = self.cache.borrow().get(&page) {
                return p.get(idx).map(|o| o.clone().upcast());
            }
            let objs = self.load_page(page);
            let out = objs.get(idx).map(|o| o.clone().upcast());
            let mut cache = self.cache.borrow_mut();
            if cache.len() >= MAX_PAGES {
                cache.clear();
            }
            cache.insert(page, objs);
            out
        }
    }

    impl ReleaseModel {
        fn load_page(&self, page: u32) -> Vec<RowObject> {
            let ids = self.ids.borrow();
            let start = (page * PAGE) as usize;
            let end = (start + PAGE as usize).min(ids.len());
            let slice = &ids[start..end];
            let base = self.base.borrow();
            let Some(base) = base.as_ref() else {
                return Vec::new();
            };
            let rows = base.rows(slice).unwrap_or_else(|e| {
                tracing::warn!("no se pudieron leer filas: {e}");
                Vec::new()
            });
            let marks: HashMap<i64, Mark> = base
                .user()
                .marks_for(slice)
                .unwrap_or_default()
                .into_iter()
                .collect();
            // Un id sin fila (no debería pasar) deja un hueco con fila vacía para no desplazar
            // las posiciones.
            let mut by_id: HashMap<i64, Row> = rows.into_iter().map(|r| (r.id, r)).collect();
            slice
                .iter()
                .map(|id| {
                    let row = by_id.remove(id).unwrap_or_else(|| Row {
                        id: *id,
                        ..Default::default()
                    });
                    RowObject::new(row, marks.get(id).copied())
                })
                .collect()
        }
    }
}

glib::wrapper! {
    /// Resultado de una búsqueda, perezoso.
    pub struct ReleaseModel(ObjectSubclass<model_imp::ReleaseModel>)
        @implements gio::ListModel;
}

impl Default for ReleaseModel {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ReleaseModel {
    /// Cambia la base (vacía la lista).
    pub fn set_base(&self, base: Option<Rc<Base>>) {
        self.imp().base.replace(base);
        self.set_ids(Vec::new());
    }

    /// Sustituye el resultado.
    pub fn set_ids(&self, ids: Vec<i64>) {
        let old = self.imp().ids.borrow().len() as u32;
        self.imp().cache.borrow_mut().clear();
        let new = ids.len() as u32;
        self.imp().ids.replace(ids);
        self.items_changed(0, old, new);
    }

    /// Ids del resultado.
    pub fn ids(&self) -> std::cell::Ref<'_, Vec<i64>> {
        self.imp().ids.borrow()
    }

    /// Posición de un id en el resultado.
    pub fn position_of(&self, id: i64) -> Option<u32> {
        self.imp()
            .ids
            .borrow()
            .iter()
            .position(|&x| x == id)
            .map(|p| p as u32)
    }

    /// Actualiza el marcador de un disco y repinta su fila.
    pub fn set_mark(&self, id: i64, mark: Option<Mark>) {
        let Some(pos) = self.position_of(id) else {
            return;
        };
        let cached = self
            .imp()
            .cache
            .borrow()
            .get(&(pos / PAGE))
            .and_then(|p| p.get((pos % PAGE) as usize).cloned());
        if let Some(obj) = cached {
            obj.imp().mark.set(mark);
        }
        self.repaint(pos);
    }

    /// Repinta la fila de un disco (p. ej. al empezar o acabar de sonar).
    pub fn refresh_row(&self, id: i64) {
        if let Some(pos) = self.position_of(id) {
            self.repaint(pos);
        }
    }

    /// La `ColumnView` no vuelve a pintar una fila si recibe el mismo objeto: se saca su página
    /// de la caché para que la siguiente lectura cree objetos nuevos.
    fn repaint(&self, pos: u32) {
        self.imp().cache.borrow_mut().remove(&(pos / PAGE));
        self.items_changed(pos, 1, 1);
    }

    /// Repinta todas las filas (p. ej. tras cambiar de tema o de disco en reproducción).
    pub fn refresh(&self) {
        let n = self.n_items();
        self.imp().cache.borrow_mut().clear();
        self.items_changed(0, n, n);
    }
}
