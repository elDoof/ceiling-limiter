#!/usr/bin/env bash
# Builds and installs Ceiling Limiter as a Linux CLAP (and VST3) plugin.
# Run this on the Linux machine from the project folder: ./build-linux.sh
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "error: run this script on Linux (current OS: $(uname -s))" >&2
    exit 1
fi

install_deps() {
    if command -v apt-get >/dev/null; then
        sudo apt-get update
        sudo apt-get install -y build-essential curl pkg-config libgl-dev libx11-dev \
            libx11-xcb-dev libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev libxcursor-dev \
            libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev libasound2-dev libjack-dev
    elif command -v dnf >/dev/null; then
        sudo dnf install -y gcc curl pkgconf-pkg-config mesa-libGL-devel libX11-devel \
            libxcb-devel xcb-util-wm-devel libXcursor-devel libxkbcommon-devel \
            alsa-lib-devel pipewire-jack-audio-connection-kit-devel
    elif command -v pacman >/dev/null; then
        sudo pacman -S --needed --noconfirm base-devel curl libgl libx11 libxcb xcb-util-wm \
            libxcursor libxkbcommon alsa-lib jack2
    else
        echo "warning: unknown package manager; install X11/XCB/OpenGL dev headers manually" >&2
    fi
}

install_rust() {
    if ! command -v cargo >/dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi
}

install_deps
install_rust

cargo test --lib --release
cargo xtask bundle ceiling_limiter --release

mkdir -p "$HOME/.clap" "$HOME/.vst3"
cp -r "target/bundled/Ceiling Limiter.clap" "$HOME/.clap/"
cp -r "target/bundled/Ceiling Limiter.vst3" "$HOME/.vst3/"

echo
echo "Installed:"
echo "  CLAP: $HOME/.clap/Ceiling Limiter.clap"
echo "  VST3: $HOME/.vst3/Ceiling Limiter.vst3"
echo "Rescan plugins in your host."
