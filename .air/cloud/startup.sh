#!/usr/bin/env bash
# Sets up a clean Linux (Debian/Ubuntu) environment to build and run dir-stat:
# Rust toolchain + a virtual X server (Xvfb) with the libraries iced/winit/wgpu need.
set -euo pipefail

SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

export DEBIAN_FRONTEND=noninteractive
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq --no-install-recommends \
  build-essential pkg-config curl ca-certificates \
  libx11-dev libxkbcommon-dev libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 \
  libwayland-client0 libfontconfig1-dev fonts-dejavu-core \
  libvulkan1 mesa-vulkan-drivers libgl1-mesa-dri libegl1 \
  xvfb x11-utils xdotool imagemagick dbus-x11

# Rust toolchain (rustup, stable)
if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
rustc --version

# Virtual X server on :99 (idempotent)
if ! pgrep -x Xvfb >/dev/null 2>&1; then
  nohup Xvfb :99 -screen 0 1400x900x24 >/tmp/xvfb.log 2>&1 &
  sleep 1
fi
echo "Xvfb running on DISPLAY=:99"

# Prefetch crates so the first build is faster
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
(cd "$PROJECT_DIR" && cargo fetch -q) || true
mkdir -p "$PROJECT_DIR/out"
