use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Label, glib};

const APP_ID: &str = "io.github.noctabremen.Atenea";

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let placeholder = Label::builder()
        .label("Atenea")
        .css_classes(["title-1"])
        .build();

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Atenea")
        .default_width(1200)
        .default_height(800)
        .child(&placeholder)
        .build();

    window.present();
}
