//! Spotlight: a presenter spotlight for the Logitech Spotlight on GNOME (Wayland).

mod config;
mod device;
mod logging;
mod overlay;
mod settings;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use tracing::info;

use config::Config;
use overlay::Overlay;
use settings::SettingsWindow;

pub const APP_ID: &str = "io.github.horauer.Spotlight";

const USAGE: &str = "\
Usage: spotlight [OPTION]

  --background   start without opening the settings window
  --test         show the spotlight for a few seconds right away
  --help         show this help

Run it again while it is running to open the settings window.";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    let background = args.iter().any(|a| a == "--background");
    let test = args.iter().any(|a| a == "--test");

    let _log_guard = logging::init();
    info!(version = env!("CARGO_PKG_VERSION"), "Spotlight starting");

    // Prefer XWayland: on GNOME Wayland only X11 override-redirect windows can
    // stay on top without stealing focus (see overlay.rs). Falls back to Wayland.
    gdk::set_allowed_backends("x11,*");

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(move |app| startup(app, background, test));
    // We parse our own options; GApplication would reject them.
    app.run_with_args(&args[..1])
}

/// Runs once, in the first instance only.
fn startup(app: &adw::Application, background: bool, test: bool) {
    load_css();

    let config = Rc::new(RefCell::new(Config::load()));
    let overlay = Overlay::new(app, config.clone());
    let settings = SettingsWindow::new(app, config, overlay.clone());

    // Device events arrive from background threads and are handled here on the UI thread.
    let (tx, rx) = async_channel::unbounded();
    device::spawn(tx);
    let (s, o) = (settings.clone(), overlay.clone());
    glib::spawn_future_local(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                device::Event::Status(status) => s.set_status(&status),
                device::Event::Activity => o.activity(),
            }
        }
    });

    add_actions(app, &settings);

    // Keep running with no window open, so the spotlight is always available.
    std::mem::forget(app.hold());

    // Each launch (including this first one) opens the settings window,
    // except a first launch with --background.
    let skip_first = Cell::new(background);
    app.connect_activate(move |_| {
        if !skip_first.replace(false) {
            settings.present();
        }
    });

    if test {
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || overlay.test());
    }
}

fn add_actions(app: &adw::Application, settings: &SettingsWindow) {
    let quit = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| {
        info!("quitting");
        a.quit();
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    let about = gio::SimpleAction::new("about", None);
    let window = settings.window().clone();
    about.connect_activate(move |_, _| {
        adw::AboutDialog::builder()
            .application_name("Spotlight")
            .application_icon("video-display")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("A presenter spotlight for the Logitech Spotlight on GNOME.\nInspired by Projecteur.")
            .website("https://github.com/gbin/Projecteur")
            .license_type(gtk::License::MitX11)
            .build()
            .present(Some(&window));
    });
    app.add_action(&about);
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string("window.spotlight-overlay { background: none; box-shadow: none; }");
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
