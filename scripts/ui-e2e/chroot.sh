#!/bin/sh
# chroot.sh <command...>: run a command inside a PureOS Crimson chroot with the repository
# mounted at /src. Used for building against Crimson's own libraries (GTK 4.8.3, libadwaita 1.2.2,
# libsoup 3.2, libsecret 0.20) and for the headless UI checks. Needs root.
#
#   CROOT=/path/to/crimson-root  chroot.sh cargo build -p jastrzab       (inside: cd /src first)
#
# The chroot itself is made once, see README.md. Proxy variables are passed through, and a CA
# bundle given in CA_BUNDLE is copied in, for environments that need them.
set -e
: "${CROOT:?set CROOT to the Crimson chroot directory}"
repo=$(cd "$(dirname "$0")/../.." && pwd)
mountpoint -q "$CROOT/proc" || mount -t proc proc "$CROOT/proc"
mountpoint -q "$CROOT/dev" || mount --bind /dev "$CROOT/dev"
mountpoint -q "$CROOT/dev/shm" || mount -t tmpfs tmpfs "$CROOT/dev/shm"
mkdir -p "$CROOT/src"
mountpoint -q "$CROOT/src" || mount --bind "$repo" "$CROOT/src"
if [ -n "$CA_BUNDLE" ]; then
    cp "$CA_BUNDLE" "$CROOT/etc/ssl/extra-ca.crt"
    chmod 644 "$CROOT/etc/ssl/extra-ca.crt"
    ca="SSL_CERT_FILE=/etc/ssl/extra-ca.crt CARGO_HTTP_CAINFO=/etc/ssl/extra-ca.crt"
fi
exec chroot "$CROOT" /usr/bin/env -i HOME=/root LANG=C.UTF-8 \
    PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_TARGET_DIR=/root/target \
    HTTPS_PROXY="$HTTPS_PROXY" HTTP_PROXY="$HTTP_PROXY" https_proxy="$HTTPS_PROXY" http_proxy="$HTTP_PROXY" \
    SHOT="$SHOT" PHOC_KEYFILE_EXTRA="$PHOC_KEYFILE_EXTRA" EXPECT_SIZE="$EXPECT_SIZE" WITH_GEOCLUE="$WITH_GEOCLUE" \
    $ca "$@"
