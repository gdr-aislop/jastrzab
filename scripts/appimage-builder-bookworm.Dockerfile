# Build environment for scripts/build-appimage.sh (Debian bookworm).
#
# Used by scripts/build-appimage-in-docker.sh so local builds happen against
# the same library baseline as the CI builds (.github/workflows/release.yml,
# which runs the same apt install inside a debian:bookworm container job) and
# therefore against the same library versions as the target distros
# (PureOS Crimson, Ubuntu 24.04).

FROM debian:bookworm

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config git curl ca-certificates file patchelf zsync gettext \
        libgtk-4-dev libadwaita-1-dev libsoup-3.0-dev libsecret-1-dev \
        libgdk-pixbuf-2.0-dev \
        glib-networking dconf-gsettings-backend gsettings-desktop-schemas \
        librsvg2-common librsvg2-bin adwaita-icon-theme \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --profile minimal
