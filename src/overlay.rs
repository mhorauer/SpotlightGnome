//! The overlay that dims the screen except for a circle around the pointer.
//!
//! GNOME on Wayland doesn't let a background app put a window on top of others.
//! So the overlay uses XWayland "override-redirect" windows, one per monitor:
//! these always sit above everything, never take keyboard focus (Next/Previous
//! keep going to the presentation), and need no help from the window manager.
//! Each window draws the spotlight only while the pointer is on its monitor.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gdk_x11::x11::xlib;
use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use tracing::{debug, info, warn};

use crate::config::{Config, parse_color};

/// How long the "Test" button keeps the spotlight up without device movement.
const TEST_DURATION: Duration = Duration::from_secs(4);
/// How often we check whether it is time to hide the spotlight.
const HIDE_CHECK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct Overlay(Rc<Inner>);

struct Inner {
    app: adw::Application,
    config: Rc<RefCell<Config>>,
    windows: RefCell<Vec<ScreenWindow>>,
    visible: Cell<bool>,
    /// Time of the last device movement. May lie in the future to keep the
    /// spotlight up for a while (test mode).
    last_activity: Cell<Instant>,
    /// Xlib, loaded at runtime. None when not running on X11 (then the overlay
    /// falls back to a normal fullscreen window, which GNOME may keep behind others).
    xlib: Option<xlib::Xlib>,
}

/// One overlay window covering one monitor.
struct ScreenWindow {
    window: gtk::Window,
    area: SpotlightArea,
    connector: Option<String>,
    geometry: gdk::Rectangle,
}

impl Overlay {
    pub fn new(app: &adw::Application, config: Rc<RefCell<Config>>) -> Self {
        let on_x11 = gdk::Display::default().is_some_and(|d| d.is::<gdk_x11::X11Display>());
        let xlib = if on_x11 {
            xlib::Xlib::open()
                .inspect_err(|e| warn!(error = %e, "cannot load libX11"))
                .ok()
        } else {
            warn!("not running on X11/XWayland; the spotlight may appear behind other windows");
            None
        };

        let overlay = Self(Rc::new(Inner {
            app: app.clone(),
            config,
            windows: RefCell::default(),
            visible: Cell::new(false),
            last_activity: Cell::new(Instant::now()),
            xlib,
        }));
        // Create the windows now, so the first show doesn't pay for GPU setup.
        overlay.sync_windows();
        overlay
    }

    /// The device moved: show the spotlight (if hidden) and keep it up.
    pub fn activity(&self) {
        let now = Instant::now();
        if self.0.last_activity.get() < now {
            self.0.last_activity.set(now);
        }
        self.show();
    }

    /// Shows the spotlight for a few seconds, without needing the device.
    pub fn test(&self) {
        let delay = Duration::from_millis(self.0.config.borrow().hide_delay_ms);
        self.0
            .last_activity
            .set(Instant::now() + TEST_DURATION.saturating_sub(delay));
        self.show();
    }

    /// Redraw after the settings changed.
    pub fn refresh(&self) {
        for sw in self.0.windows.borrow().iter() {
            sw.area.queue_draw();
        }
    }

    fn show(&self) {
        if self.0.visible.replace(true) {
            return;
        }
        let t = Instant::now();
        self.sync_windows();

        let wanted = self.0.config.borrow().monitor.clone();
        let windows = self.0.windows.borrow();
        // A chosen screen that is unplugged right now means: use all screens.
        let only = wanted.filter(|m| windows.iter().any(|sw| sw.connector.as_ref() == Some(m)));
        for sw in windows.iter() {
            if only.is_some() && sw.connector != only {
                continue;
            }
            self.present(sw);
        }
        drop(windows);
        info!(took_ms = t.elapsed().as_secs_f64() * 1000.0, "spotlight shown");

        // Hide again once the device has been still for hide_delay_ms.
        let this = self.clone();
        glib::timeout_add_local(HIDE_CHECK_INTERVAL, move || {
            if !this.0.visible.get() {
                return glib::ControlFlow::Break;
            }
            let delay = Duration::from_millis(this.0.config.borrow().hide_delay_ms);
            let idle = Instant::now().saturating_duration_since(this.0.last_activity.get());
            if idle >= delay {
                this.hide();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn present(&self, sw: &ScreenWindow) {
        match &self.0.xlib {
            Some(xlib) => {
                sw.window.set_visible(true);
                // Place it exactly over its monitor (the window manager won't do it for us).
                with_x11(xlib, &sw.window, |dpy, xid| unsafe {
                    let g = &sw.geometry;
                    (xlib.XMoveResizeWindow)(dpy, xid, g.x(), g.y(), g.width() as u32, g.height() as u32);
                    (xlib.XRaiseWindow)(dpy, xid);
                    (xlib.XFlush)(dpy);
                });
            }
            None => {
                if let Some(monitor) = find_monitor(sw.connector.as_deref().unwrap_or_default()) {
                    sw.window.fullscreen_on_monitor(&monitor);
                } else {
                    sw.window.fullscreen();
                }
                sw.window.present();
            }
        }
    }

    fn hide(&self) {
        if !self.0.visible.replace(false) {
            return;
        }
        for sw in self.0.windows.borrow().iter() {
            sw.window.set_visible(false);
            debug!(screen = ?sw.connector, frames = sw.area.take_frames(), "frames drawn");
            sw.area.set_pointer(None);
        }
        // Make sure a pending test timeout doesn't bring it back early.
        self.0.last_activity.set(Instant::now());
        info!("spotlight hidden");
    }

    /// Makes sure there is one window per monitor, matching the current layout.
    fn sync_windows(&self) {
        let current: Vec<(Option<String>, gdk::Rectangle)> = monitors()
            .iter()
            .map(|m| (m.connector().map(|c| c.to_string()), m.geometry()))
            .collect();
        let mut windows = self.0.windows.borrow_mut();
        let same = windows.len() == current.len()
            && windows
                .iter()
                .zip(&current)
                .all(|(sw, (c, g))| &sw.connector == c && sw.geometry == *g);
        if same {
            return;
        }

        for sw in windows.drain(..) {
            sw.window.destroy();
        }
        if self.0.xlib.is_some() {
            for (connector, geometry) in current {
                windows.push(self.create_window(connector, geometry));
            }
        } else {
            // Without X11 we can't place windows; use one and let GNOME pick the screen.
            let geometry = current.first().map_or(gdk::Rectangle::new(0, 0, 1, 1), |c| c.1);
            windows.push(self.create_window(None, geometry));
        }
        info!(screens = windows.len(), "overlay windows ready");
    }

    fn create_window(&self, connector: Option<String>, geometry: gdk::Rectangle) -> ScreenWindow {
        let area = SpotlightArea::new(self.0.config.clone());
        area.set_cursor_from_name(Some("none"));

        let window = gtk::Window::builder()
            .application(&self.0.app)
            .title("Spotlight overlay")
            .decorated(false)
            .hide_on_close(true)
            .default_width(geometry.width())
            .default_height(geometry.height())
            .child(&area)
            .build();
        window.add_css_class("spotlight-overlay");

        // Follow the pointer. "enter" fires as soon as the overlay appears under it.
        let motion = gtk::EventControllerMotion::new();
        let a = area.clone();
        motion.connect_enter(move |_, x, y| a.set_pointer(Some((x as f32, y as f32))));
        let a = area.clone();
        motion.connect_motion(move |_, x, y| a.set_pointer(Some((x as f32, y as f32))));
        let a = area.clone();
        motion.connect_leave(move |_| a.set_pointer(None));
        window.add_controller(motion);

        // A click while the spotlight is up dismisses it.
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&self.0); // weak: the window must not keep the overlay alive
        click.connect_pressed(move |_, _, _, _| {
            if let Some(inner) = weak.upgrade() {
                Overlay(inner).hide();
            }
        });
        window.add_controller(click);

        if let Some(xlib) = &self.0.xlib {
            make_override_redirect(xlib, &window);
        }
        ScreenWindow { window, area, connector, geometry }
    }
}

/// Turns the (not yet shown) window into an override-redirect window: the
/// window manager leaves it alone, so it stays on top and never takes focus.
fn make_override_redirect(xlib: &xlib::Xlib, window: &gtk::Window) {
    WidgetExt::realize(window);
    if let Some(surface) = window.surface().and_downcast::<gdk_x11::X11Surface>() {
        // GTK would otherwise wait for frame-sync replies the window manager
        // never sends for these windows, and stop redrawing.
        surface.set_frame_sync_enabled(false);
    }
    with_x11(xlib, window, |dpy, xid| unsafe {
        let mut attrs: xlib::XSetWindowAttributes = std::mem::zeroed();
        attrs.override_redirect = xlib::True;
        (xlib.XChangeWindowAttributes)(dpy, xid, xlib::CWOverrideRedirect, &mut attrs);
    });
}

/// Runs `f` with the X display and window id of a realized GTK window.
fn with_x11(_xlib: &xlib::Xlib, window: &gtk::Window, f: impl FnOnce(*mut xlib::Display, xlib::Window)) {
    let Some(surface) = window.surface().and_downcast::<gdk_x11::X11Surface>() else {
        return;
    };
    let Some(display) = surface.display().downcast::<gdk_x11::X11Display>().ok() else {
        return;
    };
    // SAFETY: the display pointer belongs to GDK and stays valid while the app runs.
    let dpy = unsafe { display.xdisplay() };
    f(dpy, surface.xid());
}

/// Connector names of all monitors, e.g. ["eDP-1", "HDMI-1"].
pub fn monitor_names() -> Vec<String> {
    monitors()
        .iter()
        .filter_map(|m| m.connector().map(|c| c.to_string()))
        .collect()
}

fn find_monitor(connector: &str) -> Option<gdk::Monitor> {
    monitors()
        .into_iter()
        .find(|m| m.connector().as_deref() == Some(connector))
}

fn monitors() -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let list = display.monitors();
    (0..list.n_items())
        .filter_map(|i| list.item(i).and_downcast::<gdk::Monitor>())
        .collect()
}

// ---------------------------------------------------------------------------
// SpotlightArea: a widget that draws the shade with a hole at the pointer.

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;
    use std::cell::OnceCell;

    #[derive(Default)]
    pub struct SpotlightArea {
        pub pointer: Cell<Option<(f32, f32)>>,
        pub config: OnceCell<Rc<RefCell<Config>>>,
        /// Frames drawn since the spotlight was last shown (for diagnostics).
        pub frames: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpotlightArea {
        const NAME: &'static str = "SpotlightArea";
        type Type = super::SpotlightArea;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SpotlightArea {}

    impl WidgetImpl for SpotlightArea {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some((x, y)) = self.pointer.get() else {
                return; // pointer not on this screen: draw nothing
            };
            let Some(config) = self.config.get() else { return };
            self.frames.set(self.frames.get() + 1);

            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            draw_spotlight(snapshot, &config.borrow(), w, h, x, y);
        }
    }
}

glib::wrapper! {
    pub struct SpotlightArea(ObjectSubclass<imp::SpotlightArea>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SpotlightArea {
    fn new(config: Rc<RefCell<Config>>) -> Self {
        let area: Self = glib::Object::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        let _ = imp_of(&area).config.set(config);
        area
    }

    fn set_pointer(&self, pos: Option<(f32, f32)>) {
        let imp = imp_of(self);
        if imp.pointer.get() != pos {
            imp.pointer.set(pos);
            self.queue_draw();
        }
    }

    fn take_frames(&self) -> u32 {
        imp_of(self).frames.replace(0)
    }
}

fn imp_of(area: &SpotlightArea) -> &imp::SpotlightArea {
    use gtk::subclass::prelude::ObjectSubclassIsExt;
    area.imp()
}

/// Draws the whole effect as one radial gradient: clear inside the circle,
/// an optional ring, and the shade everywhere outside it. The GPU renders this
/// in a single pass, so moving the spotlight stays smooth on large screens.
fn draw_spotlight(snapshot: &gtk::Snapshot, config: &Config, w: f32, h: f32, x: f32, y: f32) {
    const AA: f32 = 1.0; // soft edge width in pixels, avoids jagged circles

    let mut shade = parse_color(&config.shade_color, gdk::RGBA::BLACK);
    shade.set_alpha(config.shade_opacity.clamp(0.0, 1.0) as f32);
    let border = parse_color(&config.border_color, gdk::RGBA::WHITE);
    let border_w = config.border_width.max(0.0) as f32;

    let radius = (h * config.size_percent as f32 / 100.0 / 2.0).max(AA * 2.0);
    let ring_color = if border_w > 0.0 { border } else { shade };
    // Use the ring color with zero alpha inside, so the edge blends without a dark fringe.
    let mut clear = ring_color;
    clear.set_alpha(0.0);

    let outer = radius + border_w + AA;
    let at = |px: f32| (px / outer).clamp(0.0, 1.0);
    let stops = [
        gsk::ColorStop::new(0.0, clear),
        gsk::ColorStop::new(at(radius - AA), clear),
        gsk::ColorStop::new(at(radius), ring_color),
        gsk::ColorStop::new(at(radius + border_w), ring_color),
        gsk::ColorStop::new(1.0, shade),
    ];
    snapshot.append_radial_gradient(
        &graphene::Rect::new(0.0, 0.0, w, h),
        &graphene::Point::new(x, y),
        outer,
        outer,
        0.0,
        1.0,
        &stops,
    );

    if config.center_dot {
        let d = config.center_dot_size.max(1.0) as f32;
        let rect = graphene::Rect::new(x - d / 2.0, y - d / 2.0, d, d);
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, d / 2.0));
        snapshot.append_color(&parse_color(&config.center_dot_color, gdk::RGBA::RED), &rect);
        snapshot.pop();
    }
}
