*This document was written by an LLM (Claude). The steps were checked by building the app as
described in a PureOS Crimson (Debian 12 based) chroot, which is what the release workflow does;
double-check anything that matters before relying on it.*

# Building Jastrzab from source

The result is a single binary, `target/release/jastrzab`. It needs GTK 4.8+, libadwaita 1.2+,
libsoup 3, libsecret and (at run time) GeoClue and a Secret Service. The code is limited to the
GTK and libadwaita API that PureOS Crimson ships, so newer versions work too: the crate features
`gnome_43` (GTK 4.8) and `v1_2` (libadwaita 1.2) make the build fail if newer API is used.

## PureOS Crimson, Debian 12 (bookworm) and compatible

These are the packages the CI and release workflows install on `debian:bookworm`:

```sh
sudo apt install build-essential pkg-config git curl ca-certificates gettext \
  libgtk-4-dev libadwaita-1-dev libsoup-3.0-dev libsecret-1-dev
```

At run time it also wants `glib-networking` (TLS), `geoclue-2.0` (location) and `gnome-keyring`
(passwords):

```sh
sudo apt install glib-networking geoclue-2.0 gnome-keyring
```

Rust needs to be 1.92 or newer (the GTK bindings require it). Debian 12's own `rustc` (1.63) is
far too old; use [rustup](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable --profile minimal
. "$HOME/.cargo/env"
```

Then build and run:

```sh
git clone https://github.com/gdr-aislop/jastrzab
cd jastrzab
cargo build --release --locked -p jastrzab
./target/release/jastrzab
```

To install the files a phone needs (desktop entry, icon, metainfo), build the `.deb` instead:

```sh
cargo install cargo-deb --locked
cargo deb -p jastrzab        # target/debian/jastrzab_*.deb
```

Without the desktop entry Phosh does not list the app (it needs `X-Purism-FormFactor=...;Mobile;`),
and GeoClue cannot ask for location permission on its behalf (it needs `<app id>.desktop`).

### Building on the phone itself

It works but is slow and needs memory. If the build is killed for lack of memory, use fewer jobs
(`cargo build --release --locked -p jastrzab -j 2`) and/or add swap.

## AppImage

```sh
scripts/build-appimage-in-docker.sh              # in a debian:bookworm container (docker or podman)
scripts/build-appimage.sh                        # or directly, on a bookworm-like host with the build packages
```

Output: `build/appimage/jastrzab-<version>-<arch>.AppImage` and its `.zsync`, for the architecture of
the machine (`--arch x86_64|aarch64`; there is no cross-building: CI builds each natively). It needs
the packages the Dockerfile (`scripts/appimage-builder-bookworm.Dockerfile`) installs, plus
network access for the pinned `linuxdeploy` and the AppImage runtime. Building on bookworm is what
keeps the glibc requirement at 2.35 or lower, so the AppImage runs on Ubuntu 22.04 and newer.

To try it without FUSE: `APPIMAGE_EXTRACT_AND_RUN=1 ./jastrzab-*.AppImage`.

## Flatpak

```sh
sudo apt install flatpak flatpak-builder
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.gnome.Platform//51 org.gnome.Sdk//51 org.freedesktop.Sdk.Extension.rust-stable//26.08
scripts/gen-flatpak-cargo-sources.sh             # flatpak/cargo-sources.json from Cargo.lock (not committed)
flatpak-builder --user --install --force-clean build/flatpak-build flatpak/io.github.gdr_aislop.jastrzab.json
flatpak run io.github.gdr_aislop.jastrzab
```

The sandbox has no network, so every crate is vendored from `cargo-sources.json`; regenerate it
whenever `Cargo.lock` changes. The runtime and the Rust extension branch go together: GNOME 51 is
built on freedesktop-sdk 26.08. Releases are made by `.github/workflows/release.yml`; see
[docs/releasing.md](docs/releasing.md).

## Trying it without a phone

A debug build can replace GeoClue with a scripted position, so the GPS signal and sharing can be
tried on a desktop:

```sh
cargo build -p jastrzab
JASTRZAB_FAKE_LOCATION=52.52,13.40,0/3/50/300 ./target/debug/jastrzab    # accuracies cycle
JASTRZAB_FAKE_LOCATION=52.52,13.40,none ./target/debug/jastrzab         # never gets a fix
```

(`lat,lon[,metres[/metres...][,description]]`; a description starting with `GPS` looks like a
satellite fix, anything else like a network fix.) This does not exist in release builds.

## Running the tests

```sh
cargo test --workspace
```

No display is needed. The protocol tests against a real Hauk backend are marked `#[ignore]`; see
`crates/hauk/tests/live_server.rs` for how to run them, and `scripts/ui-e2e/README.md` for the
headless UI checks.
