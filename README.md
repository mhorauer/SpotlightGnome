# Spotlight

A presenter spotlight for the **Logitech Spotlight** on **GNOME** (Wayland), written in Rust
with GTK 4 and libadwaita. Inspired by [Projecteur](https://github.com/gbin/Projecteur),
with a smaller feature set aimed at working smoothly on a standard Fedora desktop.

Hold the top button on the Spotlight and move it: the screen dims except for a circle
around the pointer. Let go, and it disappears. Next/Previous keep working as usual.

## Features

- Spotlight overlay that follows the pointer, across all screens or on one chosen screen
- Adjustable size, dimming, color, ring and center dot, applied live
- Detects the Spotlight over the USB receiver or Bluetooth, including re-plugging
- Runs quietly in the background; launch it again to open the settings
- "Start at login" switch
- "Test" button to try the look without the device

Not included: zoom, vibration timers and button remapping. Those need screen capture
or Logitech's vendor protocol (HID++), which would add a lot of complexity.

## Install

Needs Rust plus `gtk4-devel` and `libadwaita-devel`:

```bash
sudo dnf install gtk4-devel libadwaita-devel
./install.sh
```

This builds a release binary to `~/.local/bin/spotlight`, adds "Spotlight" to the app grid,
and (with sudo, once) installs `data/70-spotlight.rules`. That udev rule lets the
logged-in user read the Spotlight's input events, and nothing else.

## Usage

```
spotlight              start (or open the settings if already running)
spotlight --background start without opening the settings
spotlight --test       show the spotlight for a few seconds
```

Settings are stored in `~/.config/spotlight/config.toml`. Logs are written to the terminal
and to `~/.local/state/spotlight/spotlight.log.YYYY-MM-DD` (last 7 days kept). Use `RUST_LOG=debug` for more detail.

## How it works

- **Device:** `src/device.rs` finds the Spotlight's pointer node in `/sys/class/input`
  and reads its raw events without grabbing it, so the pointer keeps working normally.
  The Spotlight only sends movement while its button is held, so movement means "show the spotlight".
- **Overlay:** GNOME on Wayland doesn't let a background app put a window on top of others,
  and a fullscreen window would steal keyboard focus from the presentation. So the overlay
  (`src/overlay.rs`) runs through XWayland as one *override-redirect* window per monitor.
  These stay on top, never take focus, and need no GNOME Shell extension. The effect is
  drawn as a single GPU radial gradient, so it stays smooth at 4K.
- **Settings:** `src/settings.rs`, a libadwaita window.

Because the app uses XWayland, the settings window is an X11 window too. On screens
with fractional scaling it may look slightly soft.

## Testing without a Spotlight

Any input device named like the Spotlight is detected, so a virtual one created with
`/dev/uinput` (e.g. named `SPOTLIGHT Mouse`) can drive the app end to end.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
