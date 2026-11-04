%global app_id io.github.horauer.Spotlight
# Release builds carry no debug info, so there is nothing for a -debuginfo package.
%global debug_package %{nil}

Name:           spotlight
Version:        0.1.0
Release:        1%{?dist}
Summary:        Presenter spotlight for the Logitech Spotlight on GNOME

License:        GPL-3.0-or-later
URL:            https://github.com/mhorauer/SpotlightGnome
Source0:        %{name}-%{version}.tar.gz
# Crate dependencies from `cargo vendor`, so the build needs no network.
Source1:        %{name}-%{version}-vendor.tar.gz

BuildRequires:  cargo >= 1.85
BuildRequires:  rust >= 1.85
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(libadwaita-1) >= 1.6
BuildRequires:  desktop-file-utils
BuildRequires:  systemd-rpm-macros

Requires:       gtk4 >= 4.14
Requires:       libadwaita >= 1.6
# The overlay runs through XWayland.
Recommends:     xorg-x11-server-Xwayland

%description
Hold the top button on the Logitech Spotlight and move it: the screen dims
except for a circle around the pointer. Let go, and it disappears.
Works on GNOME (Wayland) via XWayland override-redirect windows, with a
libadwaita settings window. Includes a udev rule that lets the logged-in user
read the Spotlight's input events.

%prep
%autosetup -n %{name}-%{version}
tar -xzf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<'CFG'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CFG

%build
export RUSTFLAGS="%{?build_rustflags}"
cargo build --release --locked --offline

%install
install -Dm755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm644 data/%{app_id}.desktop %{buildroot}%{_datadir}/applications/%{app_id}.desktop
install -Dm644 data/70-spotlight.rules %{buildroot}%{_udevrulesdir}/70-spotlight.rules

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{app_id}.desktop

%post
udevadm control --reload >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=input >/dev/null 2>&1 || :

%postun
udevadm control --reload >/dev/null 2>&1 || :

%files
%license LICENSE
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{app_id}.desktop
%{_udevrulesdir}/70-spotlight.rules

%changelog
* Wed Nov 04 2026 Martin Horauer <mhorauer@gmail.com> - 0.1.0-1
- First RPM package
