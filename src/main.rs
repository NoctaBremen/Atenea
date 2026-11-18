//! Atenea: explorador de vinilos sobre los dumps de Discogs.

mod ui;

use gtk::glib;
use gtk::prelude::*;

const APP_ID: &str = "io.github.noctabremen.Atenea";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ATENEA_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let app = gtk::Application::builder().application_id(APP_ID).build();
    // El icono se pone ya con GTK iniciado (en `startup`), no antes.
    app.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
    app.connect_activate(ui::window::build);
    // Los argumentos (una base .db) los lee la ventana; a GTK solo se le pasa el nombre.
    app.run_with_args(&["atenea"])
}
