#!/usr/bin/env bash
#
# Build an AppImage of Jastrzab for x86_64 or aarch64.
#
# Works both locally (any glibc Linux with the GTK4/libadwaita/libsoup/libsecret
# packages installed) and inside CI containers (.github/workflows/release.yml,
# debian:bookworm). The resulting AppImage bundles GTK4, libadwaita, libsoup,
# libsecret and the Adwaita icon theme, so it runs on systems that have none of
# those installed. (Adapted from the author's jastrzab script, minus GStreamer.)
#
# Usage:
#   scripts/build-appimage.sh [--arch x86_64|aarch64] [--version V] [--output-dir DIR]
#
# Environment:
#   JASTRZAB_APP_BIN    prebuilt release binary to package (skips `cargo build`)
#   LINUXDEPLOY         linuxdeploy executable to use (skips the download)
#   APPIMAGE_TOOLS_DIR  cache dir for the linuxdeploy download
#                       (default: build/appimage-tools)
#
# Bundling strategy:
#   - linuxdeploy resolves and copies every DT_NEEDED library (GTK4, adw,
#     glib, libsoup, libsecret, ...) into AppDir/usr/lib and patches rpaths.
#   - Resources that are dlopen()ed and therefore invisible to ldd are copied
#     by this script and handed to linuxdeploy via --deploy-deps-only so their
#     own dependencies get pulled in too: gio modules (TLS, dconf),
#     gdk-pixbuf loaders, GTK4 modules.
#   - The generated apprun-hooks/jastrzab.sh is sourced by linuxdeploy's
#     default AppRun and points the runtime at all of the above.
#
# Update information: LDAI_UPDATE_INFORMATION (read by linuxdeploy's bundled
# appimage output plugin, not a linuxdeploy CLI flag) is set to a
# gh-releases-zsync string below, so the AppImage embeds enough to be
# update-checked by AppImageUpdate/Gear Lever against this repo's GitHub
# releases, and the plugin generates the matching .zsync file alongside the
# .AppImage. Requires `zsyncmake` (the `zsync` package) on PATH.

set -euo pipefail

APP_NAME="jastrzab"
APP_ID="io.github.gdr_aislop.jastrzab"
GH_OWNER="gdr-aislop"
GH_REPO="jastrzab"
DATA="app/data" # the .desktop, metainfo and icons
LINUXDEPLOY_VERSION="1-alpha-20251107-1" # pinned for reproducible builds

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

die() { echo "error: $*" >&2; exit 1; }
warn() { echo "warning: $*" >&2; }

usage() {
    cat <<'USAGE'
Usage: scripts/build-appimage.sh [--arch x86_64|aarch64] [--version V] [--output-dir DIR]

Builds an AppImage of Jastrzab, bundling GTK4/libadwaita, libsoup, libsecret,
glib schemas, pixbuf loaders and the Adwaita icon theme.

Environment:
  JASTRZAB_APP_BIN    prebuilt release binary to package (skips `cargo build`)
  LINUXDEPLOY         linuxdeploy executable to use (skips the download)
  APPIMAGE_TOOLS_DIR  cache dir for the linuxdeploy download (default: build/appimage-tools)
USAGE
    exit 0
}

ARCH=""
VERSION=""
OUTPUT_DIR="$REPO_ROOT/build/appimage"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --arch) ARCH="${2:-}"; shift 2 ;;
        --version) VERSION="${2:-}"; shift 2 ;;
        --output-dir) OUTPUT_DIR="${2:-}"; shift 2 ;;
        --help|-h) usage ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done

# --- normalize architecture (linuxdeploy asset naming: x86_64/aarch64) ---
if [ -z "$ARCH" ]; then
    case "$(uname -m)" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) die "unsupported architecture: $(uname -m) (use --arch)" ;;
    esac
fi
case "$ARCH" in
    x86_64) LINUXDEPLOY_ASSET="linuxdeploy-x86_64.AppImage" ;;
    aarch64) LINUXDEPLOY_ASSET="linuxdeploy-aarch64.AppImage" ;;
    *) die "unsupported --arch: $ARCH (use x86_64 or aarch64)" ;;
esac

# --- determine version: flag > git describe > Cargo.toml workspace version ---
if [ -z "$VERSION" ]; then
    VERSION="$(git describe --tags --always --dirty 2>/dev/null || true)"
fi
if [ -z "$VERSION" ]; then
    VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)"/\1/')"
fi
[ -n "$VERSION" ] || die "cannot determine version (pass --version)"
VERSION="${VERSION//\//-}" # keep it filename-safe (branch names can contain /)

# --- tool and source availability checks ---
# zsyncmake generates the .zsync file alongside the AppImage (see the update-information note
# above) — the appimage output plugin shells out to it once LDAI_UPDATE_INFORMATION is set.
for tool in cargo pkg-config glib-compile-schemas curl zsyncmake rsvg-convert; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done

# gdk-pixbuf-query-loaders is not on $PATH on Debian/Ubuntu (it ships in
# libgdk-pixbuf-2.0-0 under the multiarch libdir); resolve it explicitly.
if command -v gdk-pixbuf-query-loaders >/dev/null 2>&1; then
    PIXBUF_QUERY="$(command -v gdk-pixbuf-query-loaders)"
else
    PIXBUF_QUERY="$(pkg-config --variable=libdir gdk-pixbuf-2.0 2>/dev/null || true)/gdk-pixbuf-2.0/gdk-pixbuf-query-loaders"
fi
[ -x "$PIXBUF_QUERY" ] || die "gdk-pixbuf-query-loaders not found (is libgdk-pixbuf-2.0-0 installed?)"

# Resolve system paths via pkg-config (works on Debian/Ubuntu layouts and others).
pkg_var() { # <variable> <package> <fallback>
    local value
    value="$(pkg-config --variable="$1" "$2" 2>/dev/null || true)"
    echo "${value:-$3}"
}

SCHEMAS_SRC="$(pkg_var schemasdir gio-2.0 /usr/share/glib-2.0/schemas)"
GIO_MODULES_SRC="$(pkg_var giomoduledir gio-2.0 '')"
PIXBUF_MODULES_SRC="$(pkg_var gdk_pixbuf_moduledir gdk-pixbuf-2.0 '')"
GTK4_MODULES_SRC="$(pkg_var libdir gtk4 '')/gtk-4.0"
[ -d "$SCHEMAS_SRC" ] || die "glib schemas dir not found ($SCHEMAS_SRC); is libglib2.0-dev installed?"
[ -d "$GIO_MODULES_SRC" ] || die "gio modules dir not found; check gio-2.0.pc / libglib2.0-dev"
[ -d "$PIXBUF_MODULES_SRC" ] || die "gdk-pixbuf loaders dir not found; check gdk-pixbuf-2.0.pc / libgdk-pixbuf-2.0-dev"

# --- build the app (unless a prebuilt binary was supplied) ---
BIN="${JASTRZAB_APP_BIN:-${CARGO_TARGET_DIR:-target}/release/jastrzab}"
if [ -z "${JASTRZAB_APP_BIN:-}" ]; then
    cargo build --release --locked -p jastrzab
fi
[ -x "$BIN" ] || die "binary not found at $BIN (set JASTRZAB_APP_BIN to override)"

# --- fetch linuxdeploy (x86_64 and aarch64 assets are published upstream) ---
TOOLS_DIR="${APPIMAGE_TOOLS_DIR:-$REPO_ROOT/build/appimage-tools}"
LINUXDEPLOY_BIN="${LINUXDEPLOY:-$TOOLS_DIR/$LINUXDEPLOY_ASSET}"
if [ ! -x "$LINUXDEPLOY_BIN" ]; then
    mkdir -p "$TOOLS_DIR"
    curl -fsSL --retry 3 -o "$LINUXDEPLOY_BIN" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_VERSION/$LINUXDEPLOY_ASSET"
    chmod +x "$LINUXDEPLOY_BIN"
fi

# --- assemble the AppDir ---
APPDIR="$OUTPUT_DIR/$APP_NAME.AppDir"
rm -rf "$APPDIR"
mkdir -p "$OUTPUT_DIR"
# Clear previous outputs so the post-build rename can't pick up stale files.
rm -f "$OUTPUT_DIR"/*.AppImage "$OUTPUT_DIR"/*.zsync

install -Dm755 "$BIN" "$APPDIR/usr/bin/$APP_NAME"
if command -v strip >/dev/null 2>&1; then
    strip "$APPDIR/usr/bin/$APP_NAME"
fi
install -Dm644 "$DATA/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$DATA/icons/hicolor/scalable/apps/$APP_ID.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$DATA/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" "$APPDIR/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
# A PNG rendering too: the AppImage's .DirIcon (its thumbnail in file managers and AppImage
# catalogs) should be a PNG, which not every thumbnailer can make from an SVG.
ICON_PNG="$APPDIR/usr/share/icons/hicolor/256x256/apps/$APP_ID.png"
mkdir -p "$(dirname "$ICON_PNG")"
rsvg-convert -w 256 -h 256 -o "$ICON_PNG" "$DATA/icons/hicolor/scalable/apps/$APP_ID.svg"
# appimagetool makes .DirIcon from the root-level icon named after the desktop file's Icon=,
# preferring a PNG; linuxdeploy only puts the SVG there, so add the PNG next to it.
ln -s "usr/share/icons/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
# The metainfo goes in under the legacy .appdata.xml name, which AppStream still reads: AppImageHub
# builds its catalog entry (description, screenshots) only from *appdata.xml, and its linter looks
# for no other name. The Flatpak and the .deb keep the current .metainfo.xml name.
METAINFO="$APPDIR/usr/share/metainfo/$APP_ID.appdata.xml"
install -Dm644 "$DATA/$APP_ID.metainfo.xml" "$METAINFO"
# appimagetool validates an .appdata.xml with appstreamcli, URL checks included, and fails on any
# warning, so an unreachable homepage would fail the build (LDAI_NO_APPSTREAM, below). Check it
# here without the network instead, where appstreamcli is installed.
if command -v appstreamcli >/dev/null 2>&1; then
    appstreamcli validate --no-net "$METAINFO" || die "AppStream metainfo does not validate"
fi
install -Dm644 LICENSE "$APPDIR/usr/share/doc/$APP_NAME/LICENSE"

# gio modules: TLS (glib-networking) and the dconf GSettings backend.
mkdir -p "$APPDIR/usr/lib/gio/modules"
for module in libgiognutls.so libgiolibproxy.so libgiognomeproxy.so libdconfsettings.so; do
    if [ -f "$GIO_MODULES_SRC/$module" ]; then
        cp -a "$GIO_MODULES_SRC/$module" "$APPDIR/usr/lib/gio/modules/"
    else
        warn "gio module not found: $module"
    fi
done

# gdk-pixbuf loaders (png/jpeg/svg covers): copy the loaders and generate the
# cache with bare file names, then symlink the loaders into usr/lib so the
# names resolve through LD_LIBRARY_PATH (same trick as linuxdeploy-plugin-gtk;
# non-relocatable gdk-pixbuf builds resolve relative cache paths via dlopen).
PIXBUF_CACHE="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
PIXBUF_MODULES_DST="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"
mkdir -p "$PIXBUF_MODULES_DST"
cp -a "$PIXBUF_MODULES_SRC"/*.so "$PIXBUF_MODULES_DST/"
(cd "$PIXBUF_MODULES_DST" && "$PIXBUF_QUERY" ./*.so) \
    | sed -e "s|$PIXBUF_MODULES_DST/||g" -e 's|\./||g' > "$PIXBUF_CACHE"
for loader in "$PIXBUF_MODULES_DST"/*.so; do
    ln -sf "gdk-pixbuf-2.0/2.10.0/loaders/$(basename "$loader")" "$APPDIR/usr/lib/"
done

# GTK4 modules (immodules, print backends); found via GTK_PATH at runtime.
if [ -d "$GTK4_MODULES_SRC" ]; then
    cp -a "$GTK4_MODULES_SRC" "$APPDIR/usr/lib/gtk-4.0"
    # The app never prints, and the CUPS backend is the only thing pulling in libcups (and its
    # avahi tail) — whose bookworm build is also the only library needing glibc 2.36, newer
    # than the 2.35 everything else needs (Ubuntu 22.04).
    find "$APPDIR/usr/lib/gtk-4.0" -name 'libprintbackend-cups.so' -delete
else
    warn "GTK4 module dir not found at $GTK4_MODULES_SRC; skipping"
fi

# glib settings schemas, compiled against the AppDir copy.
mkdir -p "$APPDIR/usr/share/glib-2.0"
cp -a "$SCHEMAS_SRC" "$APPDIR/usr/share/glib-2.0/schemas"
glib-compile-schemas "$APPDIR/usr/share/glib-2.0/schemas"

# Icon theme: the app uses -symbolic icons from Adwaita; ship the theme and
# the hicolor index so fallback lookups work without /usr/share on the host.
if [ -d /usr/share/icons/Adwaita ]; then
    cp -a /usr/share/icons/Adwaita "$APPDIR/usr/share/icons/Adwaita"
else
    warn "Adwaita icon theme not found; UI icons will depend on the host"
fi
if [ -f /usr/share/icons/hicolor/index.theme ]; then
    cp -a /usr/share/icons/hicolor/index.theme "$APPDIR/usr/share/icons/hicolor/index.theme"
fi

# Runtime environment: sourced by linuxdeploy's default AppRun.
mkdir -p "$APPDIR/apprun-hooks"
cat > "$APPDIR/apprun-hooks/$APP_NAME.sh" <<'EOF'
#!/usr/bin/env bash
# The AppImage runtime sets APPDIR; an extracted AppImage (squashfs-root/AppRun, the usual
# workaround without FUSE) doesn't, and this file sits one level below the AppDir root.
APPDIR="${APPDIR:-"$(readlink -f "$(dirname "${BASH_SOURCE[0]}")/..")"}"
export APPDIR

# Everything below points the app at the bundle — but apps it launches on the host (the log
# folder's file manager) must not inherit that, or they load our libraries
# instead of their own and die ("symbol lookup error: ... g_once_init_leave_pointer").
# Record the host's values first (JASTRZAB_HOST_<VAR>, absent if the variable was unset);
# app/src/host_env.rs puts them back for those launches. Kept from an earlier run of this
# hook, if any, so the values recorded are always the host's.
if [ -z "${JASTRZAB_BUNDLE_ENV+x}" ]; then
    export JASTRZAB_BUNDLE_ENV="LD_LIBRARY_PATH GTK_PATH GTK_EXE_PREFIX GSETTINGS_SCHEMA_DIR GDK_PIXBUF_MODULE_FILE GDK_PIXBUF_MODULEDIR GIO_MODULE_DIR XDG_DATA_DIRS"
    for var in $JASTRZAB_BUNDLE_ENV; do
        if [ -n "${!var+x}" ]; then export "JASTRZAB_HOST_$var=${!var}"; fi
    done
fi

export LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GTK_PATH="$APPDIR/usr/lib/gtk-4.0"
export GTK_EXE_PREFIX="$APPDIR/usr" # keep module lookups (immodules, media) inside the bundle
export GSETTINGS_SCHEMA_DIR="$APPDIR/usr/share/glib-2.0/schemas"
export GDK_PIXBUF_MODULE_FILE="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
export GDK_PIXBUF_MODULEDIR="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"
export GIO_MODULE_DIR="$APPDIR/usr/lib/gio/modules"
export XDG_DATA_DIRS="$APPDIR/usr/share:/usr/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
EOF
chmod +x "$APPDIR/apprun-hooks/$APP_NAME.sh"

# --- let linuxdeploy resolve and bundle shared libraries, then package ---
export APPIMAGE_EXTRACT_AND_RUN=1 # run AppImage tooling without FUSE (containers)
export VERSION="$VERSION"         # read by the appimage output plugin

DEPLOY_ARGS=()
for d in "$APPDIR/usr/lib/gio/modules" "$APPDIR/usr/lib/gdk-pixbuf-2.0" "$APPDIR/usr/lib/gtk-4.0"; do
    if [ -d "$d" ]; then
        DEPLOY_ARGS+=(--deploy-deps-only "$d")
    fi
done

# HarfBuzz is on the AppImage excludelist that linuxdeploy follows, so by default the bundled
# pango (from the build distro) runs against the host's HarfBuzz — and bookworm's pango 1.50
# needs HarfBuzz >= 4.0, so on Ubuntu 22.04 (2.7.4) and the like the app died at startup with
# "undefined symbol: hb_ot_layout_get_horizontal_baseline_tag_for_script". --library deploys it
# regardless of the excludelist. (Its own dependencies — glib, graphite2 — are bundled already;
# freetype stays the host's, and 22.04's is new enough.)
HARFBUZZ_LIB=""
for candidate in /usr/lib/*/libharfbuzz.so.0 /usr/lib/libharfbuzz.so.0; do
    if [ -e "$candidate" ]; then HARFBUZZ_LIB="$candidate"; break; fi
done
[ -n "$HARFBUZZ_LIB" ] || die "libharfbuzz.so.0 not found in the build environment"
DEPLOY_ARGS+=(--library "$HARFBUZZ_LIB")

OUTPUT_PATH="$OUTPUT_DIR/$APP_NAME-$VERSION-$ARCH.AppImage"

# The glob must match the *final* (post-rename, below) filename this script publishes, since
# that's what actually lands on the GitHub release AppImageUpdate resolves this against.
export LDAI_UPDATE_INFORMATION="gh-releases-zsync|$GH_OWNER|$GH_REPO|latest|$APP_NAME-*-$ARCH.AppImage.zsync"
# The metainfo was validated offline above.
export LDAI_NO_APPSTREAM=1

(
    cd "$OUTPUT_DIR"
    "$LINUXDEPLOY_BIN" \
        --appdir="$APPDIR" \
        --executable="$APPDIR/usr/bin/$APP_NAME" \
        --desktop-file="$APPDIR/usr/share/applications/$APP_ID.desktop" \
        --icon-file="$APPDIR/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
        --icon-file="$ICON_PNG" \
        "${DEPLOY_ARGS[@]}" \
        --output=appimage
)

# The appimage plugin derives the output file name from the desktop file's
# Name (e.g. "Jastrzab-<ver>-<arch>.AppImage"); normalize to ours. The
# output dir was cleared beforehand, so exactly one file can be present.
produced=""
for f in "$OUTPUT_DIR"/*.AppImage; do
    [ -f "$f" ] || continue
    if [ -n "$produced" ]; then
        die "multiple AppImages produced in $OUTPUT_DIR"
    fi
    produced="$f"
done
[ -n "$produced" ] || die "no AppImage produced in $OUTPUT_DIR"

# LDAI_UPDATE_INFORMATION above makes the appimage plugin shell out to zsyncmake and drop
# "<produced AppImage name>.zsync" next to it — same rename need as the AppImage itself.
produced_zsync="$produced.zsync"
[ -f "$produced_zsync" ] || die "no .zsync produced alongside $produced (is zsyncmake on PATH?)"

# libsoup and libsecret are linked directly (soup3, libsecret crates), so linuxdeploy bundles them
# as DT_NEEDED libraries; make sure they really landed, since a host without them would otherwise
# fail to start (or, for libsoup, to reach the server).
for lib in libsoup-3.0.so.0 libsecret-1.so.0; do
    [ -e "$APPDIR/usr/lib/$lib" ] || die "$lib did not land in the AppDir"
done
# Same for HarfBuzz: without it the app doesn't start on hosts with an older one.
[ -e "$APPDIR/usr/lib/libharfbuzz.so.0" ] \
    || die "libharfbuzz.so.0 did not land in the AppDir; the app wouldn't start on Ubuntu 22.04"

if [ "$produced" != "$OUTPUT_PATH" ]; then
    mv "$produced" "$OUTPUT_PATH"
    mv "$produced_zsync" "$OUTPUT_PATH.zsync"

    # zsyncmake wrote the .zsync file's Filename/URL header fields against the pre-rename name
    # (e.g. Jastrzab-<ver>-<arch>.AppImage) — the name that's actually published is the one
    # we just renamed to. Left alone, a zsync client would resolve updates against a filename that
    # was never uploaded. Patch only the ASCII header (up to the first blank line); the checksum
    # block data that follows is binary and must not be touched.
    old_basename="$(basename "$produced")"
    new_basename="$(basename "$OUTPUT_PATH")"
    sed -i "0,/^\$/{s|^Filename: .*|Filename: $new_basename|; s|^URL: .*|URL: $new_basename|}" "$OUTPUT_PATH.zsync"
    if ! grep -q "^Filename: $new_basename\$" "$OUTPUT_PATH.zsync"; then
        die "failed to rewrite $OUTPUT_PATH.zsync's header from $old_basename to $new_basename"
    fi
fi

echo
echo "Built $OUTPUT_PATH ($(du -h "$OUTPUT_PATH" | cut -f1))"
sha256sum "$OUTPUT_PATH"
echo "Built $OUTPUT_PATH.zsync (for AppImageUpdate/Gear Lever)"
sha256sum "$OUTPUT_PATH.zsync"
