//! The settings window (what you see when you launch the app).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio};
use tracing::{info, warn};

use crate::config::{Config, parse_color};
use crate::device::Status;
use crate::overlay::{self, Overlay};

const UDEV_HINT: &str = "Run ./install.sh from the project folder, or:\nsudo cp data/70-spotlight.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger";

/// Applies one settings change: updates the shared config, saves it and redraws.
type Apply = Rc<dyn Fn(&dyn Fn(&mut Config))>;

#[derive(Clone)]
pub struct SettingsWindow {
    window: adw::ApplicationWindow,
    status_row: adw::ActionRow,
    status_icon: gtk::Image,
}

impl SettingsWindow {
    pub fn new(app: &adw::Application, config: Rc<RefCell<Config>>, overlay: Overlay) -> Self {
        // Every change: update the shared config, save it, redraw the overlay.
        let apply: Apply = {
            let (config, overlay) = (config.clone(), overlay.clone());
            Rc::new(move |change: &dyn Fn(&mut Config)| {
                change(&mut config.borrow_mut());
                config.borrow().save();
                overlay.refresh();
            })
        };
        let current = config.borrow().clone();

        // --- Device ---
        let status_icon = gtk::Image::from_icon_name("system-search-symbolic");
        let status_row = adw::ActionRow::builder()
            .title("Logitech Spotlight")
            .subtitle("Looking for the device…")
            .subtitle_selectable(true)
            .build();
        status_row.add_prefix(&status_icon);

        let test_button = gtk::Button::builder()
            .label("Test")
            .tooltip_text("Show the spotlight for a few seconds. Click or press a key to close it.")
            .valign(gtk::Align::Center)
            .build();
        let o = overlay.clone();
        test_button.connect_clicked(move |_| o.test());
        status_row.add_suffix(&test_button);

        let device_group = adw::PreferencesGroup::builder()
            .title("Device")
            .description("Hold the top button on the Spotlight and move it to show the spotlight.")
            .build();
        device_group.add(&status_row);

        // --- Appearance ---
        let appearance = adw::PreferencesGroup::builder().title("Appearance").build();

        let size = spin_row("Size", "Diameter as a percentage of the screen height", 5.0, 80.0, 1.0, 0, current.size_percent);
        let a = apply.clone();
        size.connect_notify_local(Some("value"), move |row, _| {
            let v = row.value();
            a(&|c| c.size_percent = v);
        });
        appearance.add(&size);

        let opacity = spin_row("Dimming", "How dark the rest of the screen gets, in percent", 0.0, 100.0, 5.0, 0, current.shade_opacity * 100.0);
        let a = apply.clone();
        opacity.connect_notify_local(Some("value"), move |row, _| {
            let v = row.value() / 100.0;
            a(&|c| c.shade_opacity = v);
        });
        appearance.add(&opacity);

        appearance.add(&color_row("Dimming color", &current.shade_color, false, gdk::RGBA::BLACK, {
            let a = apply.clone();
            move |s| a(&|c| c.shade_color = s.clone())
        }));

        let border = spin_row("Ring width", "Ring around the spotlight in pixels (0 = none)", 0.0, 30.0, 1.0, 0, current.border_width);
        let a = apply.clone();
        border.connect_notify_local(Some("value"), move |row, _| {
            let v = row.value();
            a(&|c| c.border_width = v);
        });
        appearance.add(&border);

        appearance.add(&color_row("Ring color", &current.border_color, true, gdk::RGBA::WHITE, {
            let a = apply.clone();
            move |s| a(&|c| c.border_color = s.clone())
        }));

        let dot = adw::SwitchRow::builder()
            .title("Center dot")
            .subtitle("Mark the exact pointer position")
            .active(current.center_dot)
            .build();
        let a = apply.clone();
        dot.connect_notify_local(Some("active"), move |row, _| {
            let v = row.is_active();
            a(&|c| c.center_dot = v);
        });
        appearance.add(&dot);

        let dot_color = color_row("Center dot color", &current.center_dot_color, true, gdk::RGBA::RED, {
            let a = apply.clone();
            move |s| a(&|c| c.center_dot_color = s.clone())
        });
        dot.bind_property("active", &dot_color, "sensitive").sync_create().build();
        appearance.add(&dot_color);

        // --- Behavior ---
        let behavior = adw::PreferencesGroup::builder().title("Behavior").build();

        let delay = spin_row("Hide delay", "Milliseconds without movement before the spotlight disappears", 100.0, 5000.0, 100.0, 0, current.hide_delay_ms as f64);
        let a = apply.clone();
        delay.connect_notify_local(Some("value"), move |row, _| {
            let v = row.value() as u64;
            a(&|c| c.hide_delay_ms = v);
        });
        behavior.add(&delay);

        behavior.add(&monitor_row(&current, apply.clone()));
        behavior.add(&autostart_row());

        // --- Window ---
        let page = adw::PreferencesPage::new();
        page.add(&device_group);
        page.add(&appearance);
        page.add(&behavior);

        let menu = gio::Menu::new();
        menu.append(Some("About Spotlight"), Some("app.about"));
        menu.append(Some("Quit"), Some("app.quit"));
        let header = adw::HeaderBar::new();
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .tooltip_text("Main menu")
                .build(),
        );

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&page));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Spotlight")
            .default_width(520)
            .default_height(720)
            .hide_on_close(true) // closing the window keeps the app running in the background
            .content(&toolbar)
            .build();

        Self { window, status_row, status_icon }
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    pub fn set_status(&self, status: &Status) {
        let (icon, text) = match status {
            Status::Searching => (
                "system-search-symbolic",
                "Not found. Plug in the USB receiver or pair the Spotlight over Bluetooth.".to_owned(),
            ),
            Status::Connected(names) => ("emblem-ok-symbolic", format!("Connected: {}", names.join(", "))),
            Status::PermissionDenied(_) => (
                "dialog-warning-symbolic",
                format!("Found, but this app may not read it. Install the udev rule once. {UDEV_HINT}"),
            ),
        };
        self.status_icon.set_icon_name(Some(icon));
        self.status_row.set_subtitle(&text);
    }
}

fn spin_row(title: &str, subtitle: &str, min: f64, max: f64, step: f64, digits: u32, value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_digits(digits);
    row.set_value(value);
    row
}

fn color_row(title: &str, value: &str, with_alpha: bool, fallback: gdk::RGBA, on_change: impl Fn(String) + 'static) -> adw::ActionRow {
    let dialog = gtk::ColorDialog::builder().with_alpha(with_alpha).build();
    let button = gtk::ColorDialogButton::builder()
        .dialog(&dialog)
        .rgba(&parse_color(value, fallback))
        .valign(gtk::Align::Center)
        .build();
    button.connect_rgba_notify(move |b| on_change(b.rgba().to_string()));

    let row = adw::ActionRow::builder().title(title).build();
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    row
}

fn monitor_row(current: &Config, apply: Apply) -> adw::ComboRow {
    let mut names = overlay::monitor_names();
    if let Some(m) = &current.monitor
        && !names.contains(m)
    {
        names.push(m.clone()); // keep the saved choice even if it is unplugged right now
    }

    let mut labels = vec!["Automatic".to_owned()];
    labels.extend(names.iter().cloned());
    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());

    let selected = current
        .monitor
        .as_ref()
        .and_then(|m| names.iter().position(|n| n == m))
        .map_or(0, |i| i + 1);

    let row = adw::ComboRow::builder()
        .title("Screen")
        .subtitle("Automatic follows the pointer across all screens")
        .model(&model)
        .selected(selected as u32)
        .build();
    row.connect_notify_local(Some("selected"), move |row, _| {
        let choice = match row.selected() {
            0 | gtk::INVALID_LIST_POSITION => None,
            i => names.get(i as usize - 1).cloned(),
        };
        apply(&|c| c.monitor = choice.clone());
    });
    row
}

// --- Start at login (an XDG autostart entry) ---

fn autostart_path() -> PathBuf {
    gtk::glib::user_config_dir()
        .join("autostart")
        .join(format!("{}.desktop", crate::APP_ID))
}

fn autostart_row() -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title("Start at login")
        .subtitle("Run in the background so the Spotlight works right away")
        .active(autostart_path().exists())
        .build();
    row.connect_notify_local(Some("active"), |row, _| {
        let path = autostart_path();
        let result = if row.is_active() {
            let exe = std::env::current_exe().unwrap_or_else(|_| "spotlight".into());
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName=Spotlight\nComment=Presenter spotlight for the Logitech Spotlight\nExec=\"{}\" --background\nIcon=video-display\nX-GNOME-Autostart-enabled=true\n",
                exe.display()
            );
            std::fs::create_dir_all(path.parent().unwrap()).and_then(|_| std::fs::write(&path, entry))
        } else {
            std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        };
        match result {
            Ok(()) => info!(enabled = row.is_active(), path = %path.display(), "autostart updated"),
            Err(e) => warn!(error = %e, path = %path.display(), "failed to update autostart"),
        }
    });
    row
}
