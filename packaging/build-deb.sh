#!/usr/bin/env bash
# Builds the Spotlight .deb from the current git HEAD inside a Debian container
# (podman), so it links against Debian's libraries rather than the host's.
# Output goes to packaging/debbuild/. Override the base with DEBIAN_IMAGE.
set -euo pipefail
cd "$(dirname "$0")/.."

IMAGE=${DEBIAN_IMAGE:-docker.io/library/rust:1-trixie}
OUT="$PWD/packaging/debbuild"
mkdir -p "$OUT"
git archive --format=tar --prefix=src/ -o "$OUT/src.tar" HEAD

podman run --rm -v "$OUT:/out:Z" "$IMAGE" bash -euc '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
        debhelper dpkg-dev pkgconf libgtk-4-dev libadwaita-1-dev desktop-file-utils >/dev/null
    cd /tmp && tar -xf /out/src.tar && cd src
    # cargo/rustc come from the rust image, not apt, so skip the build-dep check (-d)
    dpkg-buildpackage -b -us -uc -d
    cp /tmp/*.deb /out/
'
rm -f "$OUT/src.tar"
ls -1 "$OUT"/*.deb
