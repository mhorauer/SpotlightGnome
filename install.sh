#!/usr/bin/env bash
# Builds Spotlight and installs it for the current user.
# The udev rule (so the app may read the device) needs sudo and is installed once.
set -euo pipefail
cd "$(dirname "$0")"

APP_ID=io.github.horauer.Spotlight
BIN="$HOME/.local/bin/spotlight"

cargo build --release
install -Dm755 target/release/spotlight "$BIN"

mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=.*|Exec=$BIN|" "data/$APP_ID.desktop" > "$HOME/.local/share/applications/$APP_ID.desktop"
echo "Installed $BIN and the app menu entry."

RULE=/etc/udev/rules.d/70-spotlight.rules
if cmp -s data/70-spotlight.rules "$RULE"; then
    echo "udev rule already installed."
else
    echo "Installing the udev rule to $RULE (needs sudo)..."
    sudo install -m644 data/70-spotlight.rules "$RULE"
    sudo udevadm control --reload
    sudo udevadm trigger --subsystem-match=input
    echo "udev rule installed. Re-plug the receiver if the device is not detected."
fi

echo "Done. Start \"Spotlight\" from the app grid; turn on \"Start at login\" in its settings."
