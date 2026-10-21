#!/usr/bin/env bash
# Build a local pkg-config shim over the WebKitGTK *runtime* libraries so the
# Tauri app links without `sudo apt install libwebkit2gtk-4.1-dev`.
set -euo pipefail
cd "$(dirname "$0")/.."
D="$PWD/.devlibs"; L=/usr/lib/x86_64-linux-gnu
mkdir -p "$D/pkgconfig" "$D/lib"
ver() { dpkg-query -W -f='${Version}' "$1" | cut -d- -f1; }
WK=$(ver libwebkit2gtk-4.1-0); SOUP=$(ver libsoup-3.0-0)
for l in webkit2gtk-4.1 javascriptcoregtk-4.1 soup-3.0; do ln -sf "$L/lib$l.so.0" "$D/lib/lib$l.so"; done
pc() { printf 'Name: %s\nDescription: runtime shim\nVersion: %s\nRequires: %s\nLibs: -L%s/lib -l%s\nCflags:\n' "$1" "$2" "$3" "$D" "$4" > "$D/pkgconfig/$1.pc"; }
pc libsoup-3.0 "$SOUP" "glib-2.0 gobject-2.0 gio-2.0" soup-3.0
pc javascriptcoregtk-4.1 "$WK" "glib-2.0 gobject-2.0" javascriptcoregtk-4.1
pc webkit2gtk-4.1 "$WK" "gtk+-3.0 libsoup-3.0 javascriptcoregtk-4.1" webkit2gtk-4.1
pc webkit2gtk-web-extension-4.1 "$WK" "gtk+-3.0 libsoup-3.0 javascriptcoregtk-4.1" webkit2gtk-4.1
echo "shim ready in $D"
