//! Atenea: explorador de vinilos sobre los dumps de Discogs.

mod config;
mod fsutil;
mod theme;
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
    app.connect_activate(ui::window::build);
    app.run()
}
