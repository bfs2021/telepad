#!/bin/sh
# Build the portable macOS / Linux release zip.
#   sh scripts/package-unix.sh                         # host target
#   sh scripts/package-unix.sh x86_64-apple-darwin     # cross-compile for another target
# Output: dist/telepad-<version>-<os>-<arch>.zip (+ .sha256), where <os>-<arch> matches
# what the in-app updater looks for (std::env::consts::OS / ARCH: macos-aarch64,
# macos-x86_64, linux-x86_64, linux-aarch64).
set -eu

cd "$(dirname "$0")/.."
TARGET=${1:-}

version=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)

case "$(uname -s)" in
    Darwin) os=macos ;;
    Linux) os=linux ;;
    *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac
if [ -n "$TARGET" ]; then
    arch=${TARGET%%-*}
else
    arch=$(uname -m)
fi
case "$arch" in
    arm64) arch=aarch64 ;;
    amd64) arch=x86_64 ;;
esac

name="telepad-$version-$os-$arch"
dist="$PWD/dist"
stage="$dist/$name"

if [ -n "$TARGET" ]; then
    cargo build --release --locked --target "$TARGET"
    bin="target/$TARGET/release"
else
    cargo build --release --locked
    bin="target/release"
fi

rm -rf "$stage" "$dist/$name.zip" "$dist/$name.zip.sha256"
mkdir -p "$stage/licenses"
cp "$bin/telepad" "$bin/telepad-check" "$stage/"
chmod 755 "$stage/telepad" "$stage/telepad-check"
cp LICENSE-MIT LICENSE-APACHE "$stage/licenses/"
cp crates/telepad-gui/assets/fonts/LICENSE-Phosphor.txt "$stage/licenses/Phosphor-MIT.txt"
cp crates/telepad-gui/assets/os/ATTRIBUTION.md "$stage/licenses/OS-logos.md"

cat >"$stage/config.example" <<'EOF'
# Example OpenSSH client config. Rename to "config" next to telepad
# before the first start to import these hosts, or use Hosts > Import from a file in the app.

# Description shown under the host name
Host example
  HostName 192.0.2.10
  User admin
  Port 22
  IdentityFile ~/.ssh/id_ed25519
EOF

if [ "$os" = macos ]; then
    first_run='  macOS blocks downloaded programs that are not notarized. Once, in this folder:
    xattr -dr com.apple.quarantine .
  Then start ./telepad from Terminal (or double-click it in Finder).'
else
    first_run='  Start ./telepad. File dialogs use the XDG desktop portal
  (xdg-desktop-portal, present on GNOME/KDE desktops).'
fi

# Portable marker: keeps telepad.db next to the program instead of the per-user folder.
printf '%s\n' "Keep Telepad's data (telepad.db) next to the program. Delete this file to use the per-user data folder instead." >"$stage/portable"

cat >"$stage/README.txt" <<EOF
Telepad $version - portable ($os $arch)
=============================================

Nothing is installed; delete the folder to remove it. Keep the folder somewhere
you can write to: telepad.db and telepad.ssh_config are created next to the program.

First start
$first_run
  * If a file named "config" (OpenSSH client config) is next to the program, its
    hosts are imported into telepad.db automatically.
  * Otherwise use Hosts > Add one host, or Hosts > Import from a file.

Updates
  Telepad checks https://github.com/yohannaftali/telepad/releases at start and
  every 6 h. "Update now" downloads the new zip, checks its SHA-256 and replaces
  both programs; your telepad.db and config are kept.

Requirements
  * The OpenSSH client (ssh) and key-based access to your hosts
  * A terminal emulator for "Open SSH" (Terminal.app on macOS; x-terminal-emulator,
    gnome-terminal, konsole, xfce4-terminal, alacritty, kitty or xterm on Linux)

Command line
  ./telepad-check [--db path] [--json] [alias ...]   one-off check, text or JSON
  ./telepad-check --check-update                     ask GitHub for a newer release
  ./telepad --db path                                use another database

Telepad only runs read-only commands on monitored hosts.
EOF

# zip keeps the execute bits; -X leaves out extra file attributes.
(cd "$dist" && zip -qrX "$name.zip" "$name")
if command -v sha256sum >/dev/null 2>&1; then
    hash=$(sha256sum "$dist/$name.zip" | cut -d' ' -f1)
else
    hash=$(shasum -a 256 "$dist/$name.zip" | cut -d' ' -f1)
fi
printf '%s  %s\n' "$hash" "$name.zip" >"$dist/$name.zip.sha256"

echo "zip:    $dist/$name.zip"
echo "sha256: $hash"

# macOS: also Telepad.app in a .dmg (ad-hoc signed; not notarized). No portable
# marker inside the app, so data goes to ~/Library/Application Support/Telepad.
if [ "$os" = macos ]; then
    dmgroot="$dist/$name-dmg"
    app="$dmgroot/Telepad.app"
    rm -rf "$dmgroot" "$dist/$name.dmg" "$dist/$name.dmg.sha256"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin/telepad" "$bin/telepad-check" "$app/Contents/MacOS/"
    chmod 755 "$app/Contents/MacOS/telepad" "$app/Contents/MacOS/telepad-check"
    cp -R "$stage/licenses" "$app/Contents/Resources/"
    cp "$stage/README.txt" "$stage/config.example" "$app/Contents/Resources/"
    cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Telepad</string>
  <key>CFBundleDisplayName</key><string>Telepad</string>
  <key>CFBundleIdentifier</key><string>io.github.yohannaftali.telepad</string>
  <key>CFBundleExecutable</key><string>telepad</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
    codesign --force --deep --sign - "$app"
    ln -s /Applications "$dmgroot/Applications"
    hdiutil create -volname "Telepad $version" -srcfolder "$dmgroot" -ov -format UDZO "$dist/$name.dmg" >/dev/null
    rm -rf "$dmgroot"
    dhash=$(shasum -a 256 "$dist/$name.dmg" | cut -d' ' -f1)
    printf '%s  %s\n' "$dhash" "$name.dmg" >"$dist/$name.dmg.sha256"
    echo "dmg:    $dist/$name.dmg"
    echo "sha256: $dhash"
fi
