#!/bin/sh
# Install Silas on macOS or Linux:
#   curl -fsSL https://raw.githubusercontent.com/baney75/silas/main/install.sh | sh
# macOS gets the stable channel; Linux gets nightly (the only Linux channel).
set -eu
REPO="https://github.com/baney75/silas/releases"
say() { printf '\033[1;33m›\033[0m %s\n' "$1"; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

case "$(uname -s)" in
Darwin)
  if [ "${SILAS_CHANNEL:-stable}" = nightly ]; then
    url="$REPO/download/nightly/Silas_universal.app.tar.gz"
  else
    url="$REPO/latest/download/Silas_universal.app.tar.gz"
  fi
  say "Downloading Silas for macOS"
  curl -fL# "$url" -o "$tmp/silas.tar.gz" || { echo "Could not download Silas from $url" >&2; exit 1; }
  tar -xzf "$tmp/silas.tar.gz" -C "$tmp"
  [ -d "$tmp/Silas.app" ] || { echo "The download did not contain Silas.app" >&2; exit 1; }
  osascript -e 'quit app "Silas"' >/dev/null 2>&1 || true
  dest=/Applications
  [ -w "$dest" ] || dest="$HOME/Applications"
  mkdir -p "$dest"
  # Stage beside the old copy, then swap, so a failure never leaves half an app.
  rm -rf "$dest/Silas.app.new"
  mv "$tmp/Silas.app" "$dest/Silas.app.new"
  rm -rf "$dest/Silas.app"
  mv "$dest/Silas.app.new" "$dest/Silas.app"
  xattr -dr com.apple.quarantine "$dest/Silas.app" 2>/dev/null || true
  say "Installed to $dest. Opening Silas…"
  open "$dest/Silas.app"
  ;;
Linux)
  api="https://api.github.com/repos/baney75/silas/releases/tags/nightly"
  assets=$(curl -fsSL "$api" | grep -o '"browser_download_url": *"[^"]*"' | cut -d'"' -f4) || true
  [ -n "$assets" ] || { echo "The nightly release is being rebuilt. Try again in a few minutes." >&2; exit 1; }
  arch=$(uname -m); [ "$arch" = x86_64 ] || { echo "Silas builds are x86_64 only for now" >&2; exit 1; }
  if command -v apt-get >/dev/null 2>&1; then
    deb=$(echo "$assets" | grep '\.deb$' | head -1)
    say "Downloading Silas (.deb)"; curl -fL# "$deb" -o "$tmp/silas.deb"
    sudo apt-get install -y "$tmp/silas.deb"
  elif command -v dnf >/dev/null 2>&1; then
    rpm=$(echo "$assets" | grep '\.rpm$' | head -1)
    say "Downloading Silas (.rpm)"; curl -fL# "$rpm" -o "$tmp/silas.rpm"
    sudo dnf install -y "$tmp/silas.rpm"
  else
    img=$(echo "$assets" | grep '\.AppImage$' | head -1)
    mkdir -p "$HOME/.local/bin"
    say "Downloading Silas (AppImage)"; curl -fL# "$img" -o "$HOME/.local/bin/silas"
    chmod +x "$HOME/.local/bin/silas"
    say "Allowing Silas to read the hold-to-talk key (needs sudo once)"
    printf 'KERNEL=="uinput", TAG+="uaccess"\nSUBSYSTEM=="input", KERNEL=="event*", TAG+="uaccess"\n' | sudo tee /etc/udev/rules.d/70-silas-input.rules >/dev/null
    sudo udevadm control --reload && sudo udevadm trigger
  fi
  if [ -n "${WAYLAND_DISPLAY:-}" ] && ! command -v wtype >/dev/null 2>&1; then
    say "Tip: install 'wtype' so Silas can type into Wayland apps."
  fi
  say "Done. Start Silas from your app launcher."
  ;;
*) echo "On Windows run: irm https://raw.githubusercontent.com/baney75/silas/main/install.ps1 | iex" >&2; exit 1 ;;
esac
