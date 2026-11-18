//! Diálogos pequeños: pedir un texto y confirmar algo.

use std::rc::Rc;

use gtk::prelude::*;

/// Pide un texto (nombre de maleta, de búsqueda…). `on_ok` recibe el texto sin espacios de
/// sobra; si devuelve un error, se muestra y el diálogo sigue abierto.
pub fn ask_text(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    initial: &str,
    ok_label: &str,
    on_ok: impl Fn(String) -> Result<(), String> + 'static,
) {
    let win = gtk::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(360)
        .resizable(false)
        .build();
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_start(14);
    b.set_margin_end(14);
    b.set_margin_top(14);
    b.set_margin_bottom(14);
    let entry = gtk::Entry::builder()
        .text(initial)
        .activates_default(true)
        .build();
    let error = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["status-clip"])
        .visible(false)
        .build();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let ok = gtk::Button::with_label(ok_label);
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    b.append(&entry);
    b.append(&error);
    b.append(&row);
    win.set_child(Some(&b));
    win.set_default_widget(Some(&ok));

    {
        let win = win.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    let on_ok = Rc::new(on_ok);
    let submit = {
        let (win, entry, error) = (win.clone(), entry.clone(), error.clone());
        move || match on_ok(entry.text().trim().to_owned()) {
            Ok(()) => win.close(),
            Err(e) => {
                error.set_label(&e);
                error.set_visible(true);
            }
        }
    };
    {
        let s = submit.clone();
        ok.connect_clicked(move |_| s());
    }
    entry.connect_activate(move |_| submit());
    win.present();
    entry.grab_focus();
}

/// Pide confirmación para algo que no se puede deshacer.
pub fn confirm(
    parent: &impl IsA<gtk::Window>,
    message: &str,
    detail: &str,
    action: &str,
    on_yes: impl Fn() + 'static,
) {
    let d = gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .buttons(["Cancelar", action])
        .cancel_button(0)
        .default_button(0)
        .modal(true)
        .build();
    d.choose(Some(parent), gtk::gio::Cancellable::NONE, move |r| {
        if r == Ok(1) {
            on_yes();
        }
    });
}

/// Aviso con un solo botón.
pub fn inform(parent: &impl IsA<gtk::Window>, message: &str, detail: &str) {
    gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build()
        .show(Some(parent));
}
