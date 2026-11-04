#!/usr/bin/env bash
# Builds the Spotlight RPM (and SRPM) from the current git HEAD.
# Output goes to packaging/rpmbuild/{RPMS,SRPMS}.
set -euo pipefail
cd "$(dirname "$0")/.."

NAME=spotlight
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TOP="$PWD/packaging/rpmbuild"
mkdir -p "$TOP"/{SOURCES,SPECS,BUILD,RPMS,SRPMS}

# Source tarball from the committed tree
git archive --format=tar.gz --prefix="$NAME-$VERSION/" -o "$TOP/SOURCES/$NAME-$VERSION.tar.gz" HEAD

# Vendored crates, so rpmbuild can build offline
VENDOR_TMP=$(mktemp -d)
trap 'rm -rf "$VENDOR_TMP"' EXIT
cargo vendor --locked --versioned-dirs "$VENDOR_TMP/vendor" >/dev/null
tar -czf "$TOP/SOURCES/$NAME-$VERSION-vendor.tar.gz" -C "$VENDOR_TMP" vendor

cp packaging/$NAME.spec "$TOP/SPECS/"
rpmbuild --define "_topdir $TOP" -ba "$TOP/SPECS/$NAME.spec"

echo
find "$TOP/RPMS" "$TOP/SRPMS" -name '*.rpm' -print
