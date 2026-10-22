//! Finds Logitech Spotlight devices and reports when they move the pointer.
//!
//! The Spotlight only sends pointer movement while its top button is held, so
//! "the device is moving" is the signal to show the spotlight. We read the
//! device's evdev node without grabbing it, so the pointer keeps moving normally.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

/// (vendor, product) ids: the Spotlight USB receiver and the Spotlight over Bluetooth.
const KNOWN_IDS: &[(u16, u16)] = &[(0x046d, 0xc53e), (0x046d, 0xb503)];

const EV_REL: u16 = 0x02;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;

/// Don't flood the UI thread: at most one activity message per this interval.
const ACTIVITY_THROTTLE: Duration = Duration::from_millis(15);
const RESCAN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Searching,
    Connected(Vec<String>),
    /// Device found but we may not read it. Holds the device names.
    PermissionDenied(Vec<String>),
}

#[derive(Debug)]
pub enum Event {
    Status(Status),
    /// The Spotlight moved the pointer.
    Activity,
}

/// Starts the background threads that watch for Spotlight devices.
pub fn spawn(tx: async_channel::Sender<Event>) {
    std::thread::Builder::new()
        .name("device-scan".into())
        .spawn(move || scan_loop(tx))
        .expect("failed to start device thread");
}

/// Readers currently open: event node path -> device name.
type OpenDevices = Arc<Mutex<HashMap<PathBuf, String>>>;

fn scan_loop(tx: async_channel::Sender<Event>) {
    let open: OpenDevices = Arc::default();
    let mut last_status = None;

    loop {
        let mut denied = Vec::new();
        for (node, name) in find_spotlights() {
            if open.lock().unwrap().contains_key(&node) {
                continue;
            }
            match File::open(&node) {
                Ok(file) => {
                    info!(device = %name, node = %node.display(), "Spotlight connected");
                    open.lock().unwrap().insert(node.clone(), name.clone());
                    let (tx, open) = (tx.clone(), open.clone());
                    std::thread::Builder::new()
                        .name("device-read".into())
                        .spawn(move || read_loop(file, node, name, tx, open))
                        .expect("failed to start reader thread");
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied.push(name),
                Err(e) => warn!(device = %name, node = %node.display(), error = %e, "cannot open device"),
            }
        }

        let connected: Vec<String> = open.lock().unwrap().values().cloned().collect();
        let status = if !connected.is_empty() {
            Status::Connected(connected)
        } else if !denied.is_empty() {
            Status::PermissionDenied(denied)
        } else {
            Status::Searching
        };

        if last_status.as_ref() != Some(&status) {
            match &status {
                Status::Searching => info!("waiting for a Spotlight device"),
                Status::Connected(names) => info!(?names, "Spotlight ready"),
                Status::PermissionDenied(names) => {
                    warn!(?names, "Spotlight found but not readable; install the udev rule")
                }
            }
            if tx.send_blocking(Event::Status(status.clone())).is_err() {
                return; // UI is gone
            }
            last_status = Some(status);
        }

        std::thread::sleep(RESCAN_INTERVAL);
    }
}

fn read_loop(mut file: File, node: PathBuf, name: String, tx: async_channel::Sender<Event>, open: OpenDevices) {
    let size = std::mem::size_of::<libc::input_event>();
    let mut buf = vec![0u8; size * 64];
    let mut last_sent: Option<Instant> = None;

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                debug!(device = %name, error = %e, "read ended");
                break;
            }
        };

        // The kernel always hands out whole input_event structs. The event type,
        // code and value are the last 8 bytes of each one.
        let moved = buf[..n].chunks_exact(size).any(|ev| {
            let t = u16::from_ne_bytes([ev[size - 8], ev[size - 7]]);
            let code = u16::from_ne_bytes([ev[size - 6], ev[size - 5]]);
            let value = i32::from_ne_bytes([ev[size - 4], ev[size - 3], ev[size - 2], ev[size - 1]]);
            t == EV_REL && (code == REL_X || code == REL_Y) && value != 0
        });

        if moved && last_sent.is_none_or(|t| t.elapsed() >= ACTIVITY_THROTTLE) {
            last_sent = Some(Instant::now());
            if tx.send_blocking(Event::Activity).is_err() {
                break;
            }
        }
    }

    info!(device = %name, node = %node.display(), "Spotlight disconnected");
    open.lock().unwrap().remove(&node);
}

/// Returns (/dev/input/eventN, name) for every pointer node that belongs to a Spotlight.
fn find_spotlights() -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let event_name = entry.file_name().to_string_lossy().into_owned();
        if !event_name.starts_with("event") {
            continue;
        }
        let dev = entry.path().join("device"); // the parent inputN directory
        if !has_pointer_motion(&dev) {
            continue;
        }
        let name = read_trimmed(&dev.join("name")).unwrap_or_default();
        if is_spotlight(&dev, &name) {
            found.push((Path::new("/dev/input").join(&event_name), name));
        }
    }
    found
}

/// True if the device reports relative X and Y movement (i.e. it is a pointer).
fn has_pointer_motion(dev: &Path) -> bool {
    // capabilities/rel is a hex bitmask in words, most significant first;
    // REL_X and REL_Y are bits 0 and 1 of the last word.
    read_trimmed(&dev.join("capabilities/rel"))
        .and_then(|s| s.split_whitespace().last().map(str::to_owned))
        .and_then(|w| u64::from_str_radix(&w, 16).ok())
        .is_some_and(|bits| bits & 0b11 == 0b11)
}

fn is_spotlight(dev: &Path, name: &str) -> bool {
    if name.to_lowercase().contains("spotlight") {
        return true;
    }
    let input_id = (read_hex(&dev.join("id/vendor")), read_hex(&dev.join("id/product")));
    if let (Some(v), Some(p)) = input_id
        && KNOWN_IDS.contains(&(v, p))
    {
        return true;
    }
    // Devices behind the USB receiver: check the USB device further up the tree.
    let Ok(mut dir) = dev.canonicalize() else {
        return false;
    };
    while dir.pop() && dir != Path::new("/sys") {
        if let (Some(v), Some(p)) = (read_hex(&dir.join("idVendor")), read_hex(&dir.join("idProduct")))
        {
            return KNOWN_IDS.contains(&(v, p));
        }
    }
    false
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

fn read_hex(path: &Path) -> Option<u16> {
    read_trimmed(path).and_then(|s| u16::from_str_radix(&s, 16).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a fake sysfs input device directory under a fresh temp folder.
    fn fake_input(parent: &Path, rel: &str, vendor: &str, product: &str) -> PathBuf {
        let dev = parent.join("input9");
        std::fs::create_dir_all(dev.join("capabilities")).unwrap();
        std::fs::create_dir_all(dev.join("id")).unwrap();
        std::fs::write(dev.join("capabilities/rel"), format!("{rel}\n")).unwrap();
        std::fs::write(dev.join("id/vendor"), format!("{vendor}\n")).unwrap();
        std::fs::write(dev.join("id/product"), format!("{product}\n")).unwrap();
        dev
    }

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spotlight-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_pointer_capability() {
        let root = temp_root("caps");
        assert!(has_pointer_motion(&fake_input(&root.join("a"), "143", "046d", "b503")));
        assert!(has_pointer_motion(&fake_input(&root.join("b"), "0 3", "046d", "b503")));
        assert!(!has_pointer_motion(&fake_input(&root.join("c"), "0", "046d", "b503")));
        assert!(!has_pointer_motion(&fake_input(&root.join("d"), "1", "046d", "b503")));
    }

    #[test]
    fn matches_spotlight_but_not_other_logitech_devices() {
        let root = temp_root("ids");
        let bluetooth = fake_input(&root.join("bt"), "3", "046d", "b503");
        assert!(is_spotlight(&bluetooth, "SPOTLIGHT Mouse"));
        assert!(is_spotlight(&bluetooth, "some other name"));

        let mx_keys = fake_input(&root.join("mx"), "143", "046d", "408a");
        assert!(!is_spotlight(&mx_keys, "Logitech MX Keys"));
    }

    #[test]
    fn matches_device_behind_usb_receiver() {
        let root = temp_root("usb");
        let usb = root.join("usb1");
        std::fs::create_dir_all(&usb).unwrap();
        std::fs::write(usb.join("idVendor"), "046d\n").unwrap();
        std::fs::write(usb.join("idProduct"), "c53e\n").unwrap();
        // The input device's own ids are something else; the USB ancestor decides.
        let dev = fake_input(&usb.join("0003:046D:C53E.0001"), "3", "046d", "4040");
        assert!(is_spotlight(&dev, "Logitech USB Receiver Mouse"));
    }
}
