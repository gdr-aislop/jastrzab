use adw::prelude::*;
use jastrzab_core::AppPaths;
use jastrzab_core::settings::Settings;

use crate::controller::Controller;

pub fn version_line() -> String {
    format!("Jastrzab {} ({})", env!("CARGO_PKG_VERSION"), env!("JASTRZAB_GIT_HASH"))
}

pub fn build_application(paths: AppPaths) -> adw::Application {
    let app = adw::Application::builder().application_id(jastrzab_core::paths::APP_ID).build();
    let controller: std::rc::Rc<std::cell::RefCell<Option<Controller>>> = Default::default();

    app.connect_startup({
        let controller = controller.clone();
        move |app| {
            crate::i18n::init();
            load_css();
            crate::ui::icons::register();
            let settings = load_settings(&paths);
            if let Err(err) = paths.ensure_dirs() {
                tracing::warn!(%err, "couldn't create the settings and state folders");
            }
            *controller.borrow_mut() = Some(Controller::new(app, &paths, settings));
        }
    });
    app.connect_activate({
        let controller = controller.clone();
        move |_| {
            if let Some(controller) = controller.borrow().as_ref() {
                controller.present_window();
            }
        }
    });
    app.connect_shutdown(move |_| {
        if let Some(controller) = controller.borrow().as_ref() {
            controller.shutdown();
        }
    });
    app
}

fn load_settings(paths: &AppPaths) -> Settings {
    match std::fs::read_to_string(paths.settings_file()) {
        Ok(text) => Settings::from_toml(&text),
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%err, "couldn't read the settings file; using the defaults");
            }
            Settings::default()
        }
    }
}

fn load_css() {
    let Some(display) = gtk::gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("../data/style.css"));
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}
