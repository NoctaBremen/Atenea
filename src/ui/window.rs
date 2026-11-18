//! Ventana principal: barra de título con el menú de ajustes y, de momento, contenido vacío.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use atenea::config::Config;
use atenea::theme::{THEMES, Theme};

/// Estilo propio de Atenea, después del CSS del tema.
const CSS: &str = "
.placeholder-title { font-size: 28pt; font-weight: bold; }
.settings-popover { padding: 12px; }
";

/// CSS completo de la ventana para un tema ya ajustado.
pub fn css_for(theme: &Theme) -> String {
    format!("{}{CSS}", theme.gtk_css())
}

/// Índice del tema con ese id en [`THEMES`]; si no existe, el de por defecto (0).
pub fn theme_index(id: &str) -> usize {
    THEMES.iter().position(|t| t.id == id).unwrap_or(0)
}

/// Construye y muestra la ventana.
pub fn build(app: &gtk::Application) {
    let config = Rc::new(RefCell::new(Config::load()));
    let index = theme_index(&config.borrow().theme);

    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css_for(&Theme::by_id(&config.borrow().theme).effective()));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let names: Vec<&str> = THEMES.iter().map(|t| t.name).collect();
    let theme_dd = gtk::DropDown::from_strings(&names);
    theme_dd.set_selected(index as u32);
    {
        let config = Rc::clone(&config);
        let provider = provider.clone();
        theme_dd.connect_selected_notify(move |dd| {
            let Some(t) = THEMES.get(dd.selected() as usize) else {
                return;
            };
            provider.load_from_string(&css_for(&t.effective()));
            config.borrow_mut().theme = t.id.into();
            config.borrow().save();
        });
    }

    let settings_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    settings_box.add_css_class("settings-popover");
    settings_box.append(
        &gtk::Label::builder()
            .label("Tema")
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    settings_box.append(&theme_dd);

    let popover = gtk::Popover::builder().child(&settings_box).build();
    let menu = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Ajustes")
        .popover(&popover)
        .build();

    let header = gtk::HeaderBar::new();
    header.pack_end(&menu);

    let placeholder = gtk::Label::builder()
        .label("Atenea")
        .css_classes(["placeholder-title"])
        .build();

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Atenea")
        .default_width(1200)
        .default_height(800)
        .titlebar(&header)
        .child(&placeholder)
        .build();

    window.present();
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
        assert!(css.contains(".placeholder-title"));
    }
}
