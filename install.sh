#!/bin/sh
# Install Silas on macOS or Linux:
#   curl -fsSL https://raw.githubusercontent.com/baney75/silas/main/install.sh | sh
# Remove it (your dictionary and history stay):
#   curl -fsSL https://raw.githubusercontent.com/baney75/silas/main/install.sh | sh -s -- --uninstall
# macOS gets the stable channel (SILAS_CHANNEL=nightly for nightly); Linux gets nightly,
# the only Linux channel. Other knobs: SILAS_INSTALL_DIR (macOS, default /Applications),
# SILAS_LINUX_FORMAT=appimage (skip the .deb/.rpm), SILAS_NO_OPEN=1 (don't launch Silas).
set -eu
# A CDPATH from the user's shell would send `cd` to some other folder of the same name.
unset CDPATH
REPO="https://github.com/baney75/silas/releases"
RAW="https://raw.githubusercontent.com/baney75/silas/main"
UNINSTALL="curl -fsSL $RAW/install.sh | sh -s -- --uninstall"
say() { printf '\033[1;33m›\033[0m %s\n' "$1"; }
die() { printf '\033[1;31m✗\033[0m %s\n' "$1" >&2; exit 1; }
# Root shells (containers, some servers) have no sudo and don't need it.
as_root() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi; }

action=install
case "${1:-}" in
--uninstall | uninstall) action=uninstall ;;
"" | --install | install) ;;
-h | --help)
  echo "usage: install.sh [--uninstall]"
  exit 0
  ;;
*) die "Unknown option '$1'. Use --uninstall to remove Silas." ;;
esac

tmp=$(mktemp -d)
staged=""
trap 'rm -rf "$tmp"; [ -z "$staged" ] || rm -f "$staged"' EXIT
trap 'exit 130' INT TERM

# PIDs of processes whose executable path starts with $1 or $2 (the same folder with and
# without symlinks resolved). The folder is compared as a literal prefix, never a regex, and
# reaches awk through the environment, which keeps backslashes in folder names as they are.
pids_in() {
  ps -axo pid=,comm= | A="$1" B="$2" awk '{
    pid = $1; path = $0; sub(/^[ \t]*[0-9]+[ \t]+/, "", path)
    if (index(path, ENVIRON["A"]) == 1 || index(path, ENVIRON["B"]) == 1) print pid
  }'
}

quit_mac() {
  if [ -n "${SILAS_INSTALL_DIR:-}" ]; then
    # A custom folder: stop only a Silas running from that folder, never the user's main one.
    # ${x%/}: a folder of "/" must give "/Silas.app/...", not "//Silas.app/...".
    a="${SILAS_INSTALL_DIR%/}/Silas.app/Contents/MacOS/"
    b="${SILAS_INSTALL_DIR_REAL%/}/Silas.app/Contents/MacOS/"
    pids=$(pids_in "$a" "$b")
    # shellcheck disable=SC2086
    [ -z "$pids" ] || kill $pids 2>/dev/null || true
    i=0
    while [ -n "$(pids_in "$a" "$b")" ] && [ $i -lt 20 ]; do sleep 0.5; i=$((i + 1)); done
    return 0
  fi
  osascript -e 'quit app "Silas"' >/dev/null 2>&1 || true
  i=0
  while pgrep -xq silas && [ $i -lt 20 ]; do sleep 0.5; i=$((i + 1)); done
}

mac_uninstall() {
  quit_mac
  found=""
  for d in /Applications "$HOME/Applications"; do
    # With SILAS_INSTALL_DIR set, touch only that folder.
    [ -z "${SILAS_INSTALL_DIR:-}" ] || d="$SILAS_INSTALL_DIR"
    [ -d "$d/Silas.app" ] || continue
    rm -rf "$d/Silas.app" 2>/dev/null || as_root rm -rf "$d/Silas.app"
    found="$d"
  done
  [ -n "$found" ] || die "Silas isn't installed in ${SILAS_INSTALL_DIR:-/Applications or ~/Applications}."
  say "Removed Silas from $found."
  say "Your dictionary, history and voice model are still in ~/Library/Application Support/Silas."
  say "Delete that folder too if you want them gone. You can also remove Silas from"
  say "System Settings › Privacy & Security › Accessibility and Microphone."
}

mac_install() {
  # uname -m says x86_64 in a Rosetta terminal, so ask the hardware instead.
  [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ] ||
    die "Silas needs a Mac with Apple silicon (M1 or newer). Intel Macs aren't supported."
  major=$(sw_vers -productVersion | cut -d. -f1)
  [ "$major" -ge 12 ] || die "Silas needs macOS 12 Monterey or newer (this Mac runs $(sw_vers -productVersion))."
  if [ "${SILAS_CHANNEL:-stable}" = nightly ]; then
    url="$REPO/download/nightly/Silas_aarch64.app.tar.gz"
  else
    url="$REPO/latest/download/Silas_aarch64.app.tar.gz"
  fi
  say "Downloading Silas for macOS"
  curl -fL# "$url" -o "$tmp/silas.tar.gz" || die "Could not download Silas from $url. Check your connection and try again."
  tar -xzf "$tmp/silas.tar.gz" -C "$tmp"
  [ -d "$tmp/Silas.app" ] || die "The download did not contain Silas.app."
  quit_mac
  if [ -n "${SILAS_INSTALL_DIR:-}" ]; then
    dest="$SILAS_INSTALL_DIR"
    mkdir -p "$dest" || die "Can't create $dest."
    [ -w "$dest" ] || die "Can't write to $dest."
  else
    dest=/Applications
    [ -w "$dest" ] || dest="$HOME/Applications"
    mkdir -p "$dest"
  fi
  # Stage beside the old copy, then swap, so a failure never leaves half an app.
  rm -rf "$dest/Silas.app.new"
  mv "$tmp/Silas.app" "$dest/Silas.app.new"
  rm -rf "$dest/Silas.app"
  mv "$dest/Silas.app.new" "$dest/Silas.app"
  xattr -dr com.apple.quarantine "$dest/Silas.app" 2>/dev/null || true
  [ -z "${SILAS_NO_OPEN:-}" ] || { say "Installed to $dest."; return; }
  say "Installed to $dest. Opening Silas…"
  open "$dest/Silas.app" || { sleep 2; open "$dest/Silas.app"; }
  say "Allow Microphone and Accessibility when Silas asks. Then hold Right ⌘ and speak."
}

linux_desktop_entry() {
  apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"
  mkdir -p "$apps" "$icons"
  curl -fsSL "$RAW/src-tauri/icons/128x128.png" -o "$icons/silas.png" 2>/dev/null || true
  cat >"$apps/silas.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Silas
Comment=Hold a key, speak, and Silas types it.
Exec="$HOME/.local/bin/silas"
Icon=silas
Terminal=false
Categories=Utility;Accessibility;
X-Silas-Installer=true
EOF
  if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$apps" >/dev/null 2>&1 || true; fi
}

# Remove the AppImage install: binary, launcher entry, icon and udev rule, each on its own.
# True if ~/.local/bin/silas is an AppImage this script put there (AppImage magic "AI" 0x02 at
# byte 8), so a user's own script or build of that name is never deleted.
is_our_appimage() {
  f="$HOME/.local/bin/silas"
  [ -f "$f" ] && [ ! -L "$f" ] && [ "$(od -An -tx1 -j8 -N3 "$f" 2>/dev/null | tr -d ' \n')" = "414902" ]
}

remove_appimage() {
  share="${XDG_DATA_HOME:-$HOME/.local/share}"
  removed=""
  ours=""
  entry="$share/applications/silas.desktop"
  binary_ours=""
  if is_our_appimage; then
    rm -f "$HOME/.local/bin/silas" || die "Couldn't remove $HOME/.local/bin/silas."
    removed=1
    binary_ours=1
  fi
  # The launcher entry (and its icon) is ours if it carries the installer's marker, or, for
  # entries from installers before the marker, if it is exactly Name=Silas launching our
  # AppImage. Anyone's own entry or icon stays.
  if grep -qxF "X-Silas-Installer=true" "$entry" 2>/dev/null; then
    ours=1
  elif [ -n "$binary_ours" ] && grep -qxF "Name=Silas" "$entry" 2>/dev/null &&
    grep -qxF "Exec=\"$HOME/.local/bin/silas\"" "$entry" 2>/dev/null; then
    ours=1
  fi
  if [ -n "$ours" ]; then
    for f in "$share/applications/silas.desktop" "$share/icons/hicolor/128x128/apps/silas.png"; do
      if [ -e "$f" ] || [ -L "$f" ]; then
        rm -f "$f" || die "Couldn't remove $f."
        removed=1
      fi
    done
  fi
  # The udev rule only ever came from this script's AppImage install; leave it when someone
  # else's ~/.local/bin/silas is there.
  if [ -f /etc/udev/rules.d/70-silas-input.rules ] &&
    { [ -n "$binary_ours" ] || [ -n "$ours" ] || { [ ! -e "$HOME/.local/bin/silas" ] && [ ! -L "$HOME/.local/bin/silas" ]; }; }; then
    as_root rm -f /etc/udev/rules.d/70-silas-input.rules || die "Couldn't remove the udev rule."
    as_root udevadm control --reload 2>/dev/null || true
    removed=1
  fi
  [ -n "$removed" ]
}

# Stop running Silas processes that belong to this install: the package binary, or our
# AppImage (an AppImage's process carries APPIMAGE=<its file>). Another program that merely
# has the name silas is left alone.
stop_linux_silas() {
  # The AppImage runtime records the resolved path, so a symlinked home matches too.
  img="$HOME/.local/bin/silas"
  # "|| true": with no ~/.local/bin the failed cd would end the script under set -e.
  img_real="$(cd "$HOME/.local/bin" 2>/dev/null && pwd -P || true)/silas"
  for pid in $(pgrep -x silas 2>/dev/null || true); do
    exe=$(readlink "/proc/$pid/exe" 2>/dev/null || true)
    # After a package upgrade the running binary reads "/usr/bin/silas (deleted)".
    exe=${exe% (deleted)}
    case "$exe" in
    /usr/bin/silas | /bin/silas) kill "$pid" 2>/dev/null || true; continue ;;
    esac
    if tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null | grep -qxF -e "APPIMAGE=$img" -e "APPIMAGE=$img_real"; then
      kill "$pid" 2>/dev/null || true
    fi
  done
}

package_installed() {
  # dpkg keeps a removed package as "config-files"; only an installed one counts.
  if command -v dpkg-query >/dev/null 2>&1 &&
    [ "$(dpkg-query -W -f='${Status}' silas 2>/dev/null)" = "install ok installed" ]; then return 0; fi
  command -v rpm >/dev/null 2>&1 && rpm -q silas >/dev/null 2>&1
}

linux_uninstall() {
  # Stop Silas only once we know it's ours to remove.
  if package_installed || is_our_appimage; then stop_linux_silas; fi
  found=""
  if command -v dpkg-query >/dev/null 2>&1 &&
    [ "$(dpkg-query -W -f='${Status}' silas 2>/dev/null)" = "install ok installed" ]; then
    as_root apt-get remove -y silas
    found=deb
  elif command -v rpm >/dev/null 2>&1 && rpm -q silas >/dev/null 2>&1; then
    # zypper rebuilds the package's full name from its version, and the hyphen in a nightly
    # version ("0.1.7-nightly…") makes rpm miss it, so remove by name with rpm itself.
    if command -v dnf >/dev/null 2>&1; then as_root dnf remove -y silas; else as_root rpm -e silas; fi
    found=rpm
  fi
  if remove_appimage; then found=appimage; fi
  [ -n "$found" ] || die "Silas isn't installed (no silas package, and ~/.local/bin/silas is missing or isn't a Silas AppImage)."
  say "Removed Silas. Your dictionary, history and voice model are still in ${XDG_DATA_HOME:-$HOME/.local/share}/Silas."
  say "Delete that folder too if you want them gone."
}

linux_install() {
  arch=$(uname -m)
  [ "$arch" = x86_64 ] || die "Silas builds are x86_64 only for now (this machine is $arch)."
  libc=$(getconf GNU_LIBC_VERSION 2>/dev/null | cut -d' ' -f2) || libc=""
  [ -n "$libc" ] || die "Silas needs glibc 2.39 or newer, and this system doesn't use glibc."
  major=${libc%%.*}
  minor=${libc#*.}
  minor=${minor%%.*}
  if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 39 ]; }; then
    die "Silas needs glibc 2.39 or newer (Ubuntu 24.04, Fedora 40 or later). This system has $libc."
  fi

  format="${SILAS_LINUX_FORMAT:-}"
  if [ -z "$format" ]; then
    if command -v apt-get >/dev/null 2>&1; then format=deb
    elif command -v dnf >/dev/null 2>&1 || command -v zypper >/dev/null 2>&1; then format=rpm
    else format=appimage; fi
  fi
  case "$format" in
  deb | rpm | appimage) ;;
  *) die "SILAS_LINUX_FORMAT must be deb, rpm or appimage." ;;
  esac
  # Fail before any download or stopping Silas when the package tool is missing.
  if [ "$format" = deb ] && ! command -v apt-get >/dev/null 2>&1; then
    die "SILAS_LINUX_FORMAT=deb needs apt-get."
  fi
  if [ "$format" = rpm ] && ! command -v dnf >/dev/null 2>&1 && ! command -v zypper >/dev/null 2>&1; then
    die "SILAS_LINUX_FORMAT=rpm needs dnf or zypper."
  fi
  if [ "$format" = appimage ] && package_installed; then
    die "Silas is installed as a package. Remove it first: $UNINSTALL"
  fi
  if [ "$format" = appimage ] && { [ -e "$HOME/.local/bin/silas" ] || [ -L "$HOME/.local/bin/silas" ]; } && ! is_our_appimage; then
    die "$HOME/.local/bin/silas exists and isn't a Silas AppImage. Move it, then run this again."
  fi

  api="https://api.github.com/repos/baney75/silas/releases/tags/nightly"
  # SILAS_GITHUB_TOKEN (CI sets it) lifts the API's 60-requests-an-hour limit on shared IPs.
  # It goes to curl on stdin, never in argv.
  if [ -n "${SILAS_GITHUB_TOKEN:-}" ]; then
    json=$(printf 'header = "Authorization: Bearer %s"\n' "$SILAS_GITHUB_TOKEN" | curl -K - -fsSL "$api") ||
      die "Couldn't read the nightly release from GitHub with SILAS_GITHUB_TOKEN."
  else
    json=$(curl -fsSL "$api") || die "Couldn't reach GitHub ($api). Check your connection and try again."
  fi
  assets=$(echo "$json" | grep -o '"browser_download_url": *"[^"]*"' | cut -d'"' -f4) || true
  case "$format" in
  deb) url=$(echo "$assets" | grep '\.deb$' | head -1) ;;
  rpm) url=$(echo "$assets" | grep '\.rpm$' | head -1) ;;
  appimage) url=$(echo "$assets" | grep '\.AppImage$' | head -1) ;;
  esac
  [ -n "$url" ] || die "The nightly release is being rebuilt. Try again in a few minutes."

  if [ "$format" = appimage ]; then
    # Stage beside the target so the final mv is a rename on one filesystem.
    mkdir -p "$HOME/.local/bin"
    staged=$(mktemp "$HOME/.local/bin/.silas.XXXXXX")
    say "Downloading Silas (AppImage)"
    curl -fL# "$url" -o "$staged" || die "Could not download $url."
    chmod +x "$staged"
    stop_linux_silas
    mv -f "$staged" "$HOME/.local/bin/silas"
    staged=""
    linux_desktop_entry
    say "Allowing Silas to read the hold-to-talk key (needs sudo once)"
    printf 'KERNEL=="uinput", TAG+="uaccess"\nSUBSYSTEM=="input", KERNEL=="event*", TAG+="uaccess"\n' |
      as_root tee /etc/udev/rules.d/70-silas-input.rules >/dev/null
    as_root udevadm control --reload && as_root udevadm trigger
    if ! ls /usr/lib*/libfuse.so.2* /usr/lib/*/libfuse.so.2* >/dev/null 2>&1; then
      say "Tip: AppImages need FUSE 2. If Silas won't start, install your distro's 'fuse2' or 'libfuse2' package."
    fi
    bin="$HOME/.local/bin/silas"
  else
    say "Downloading Silas (.$format)"
    curl -fL# "$url" -o "$tmp/silas.$format" || die "Could not download $url."
    stop_linux_silas
    if [ "$format" = deb ]; then
      as_root apt-get install -y "$tmp/silas.deb"
    elif command -v dnf >/dev/null 2>&1; then
      as_root dnf install -y "$tmp/silas.rpm"
    else
      as_root zypper --non-interactive install --allow-unsigned-rpm "$tmp/silas.rpm"
    fi
    # An earlier AppImage install would shadow the package on PATH; the package replaces it.
    if remove_appimage; then say "Removed the earlier AppImage install; the package replaces it."; fi
    bin=/usr/bin/silas
  fi
  if [ -n "${WAYLAND_DISPLAY:-}" ] && ! command -v wtype >/dev/null 2>&1; then
    say "Tip: install 'wtype' so Silas can type into Wayland apps."
  fi
  # Never start the app as root: it would own the user's data folder and miss their session.
  if [ -z "${SILAS_NO_OPEN:-}" ] && [ "$(id -u)" != 0 ] && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    say "Done. Opening Silas…"
    nohup "$bin" >/dev/null 2>&1 &
  else
    say "Done. Start Silas from your app launcher."
  fi
  say "Then hold Right Ctrl and speak."
}

# macOS custom folder: one absolute path, so it can be compared with a running app's path.
# A quoted "~/x" isn't expanded by the shell, so expand it here; "./" keeps a name starting
# with "-" from reading as an option to cd.
if [ "$(uname -s)" = Darwin ] && [ -n "${SILAS_INSTALL_DIR:-}" ]; then
  case "$SILAS_INSTALL_DIR" in
  *"
"*) die "SILAS_INSTALL_DIR can't contain a line break." ;;
  esac
  # shellcheck disable=SC2088 # a literal "~/" is matched and expanded here on purpose
  case "$SILAS_INSTALL_DIR" in
  "~") SILAS_INSTALL_DIR=$HOME ;;
  "~/"*) SILAS_INSTALL_DIR="$HOME/${SILAS_INSTALL_DIR#"~/"}" ;;
  /*) ;;
  *) SILAS_INSTALL_DIR="./$SILAS_INSTALL_DIR" ;;
  esac
  [ "$action" = uninstall ] || mkdir -p "$SILAS_INSTALL_DIR" || die "Can't create $SILAS_INSTALL_DIR."
  [ -d "$SILAS_INSTALL_DIR" ] || die "Silas isn't installed in $SILAS_INSTALL_DIR."
  SILAS_INSTALL_DIR_REAL=$(cd "$SILAS_INSTALL_DIR" && pwd -P)
  SILAS_INSTALL_DIR=$(cd "$SILAS_INSTALL_DIR" && pwd)
fi

case "$(uname -s)" in
Darwin) "mac_$action" ;;
Linux) "linux_$action" ;;
*) die "On Windows run this in PowerShell: irm $RAW/install.ps1 | iex" ;;
esac
