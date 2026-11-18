#!/usr/bin/env bash
# Construye Vault-x86_64.AppImage con linuxdeploy y su plugin de GTK (GTK4 incluido).
#
#   packaging/build-appimage.sh            # deja el AppImage en dist/
#
# mpv y yt-dlp NO van dentro: se usan los del sistema (pacman -S mpv yt-dlp), que se
# actualizan solos y yt-dlp necesita estar al día para que YouTube funcione.
#
# Las herramientas se descargan una vez a ~/.cache/vault-appimage (o $TOOLS_DIR).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="${TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/vault-appimage}"
APPDIR="$ROOT/target/AppDir"
OUT="$ROOT/dist"
APP_ID="io.github.noctabremen.Vault"

mkdir -p "$TOOLS" "$OUT"
fetch() {
    local url="$1" file="$TOOLS/$(basename "$1")"
    if [ ! -s "$file" ]; then
        echo "Descargando $(basename "$file")…"
        curl -fsSL -o "$file" "$url"
    fi
    chmod +x "$file"
}
fetch https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
fetch https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh

echo "Compilando…"
(cd "$ROOT" && cargo build --release --bins)

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
for b in vault vault-importar vault-muestra vault-catalogo; do
    install -m755 "$ROOT/target/release/$b" "$APPDIR/usr/bin/$b"
done

# Sin FUSE (contenedores, algunas distros) las herramientas se ejecutan extraídas.
export APPIMAGE_EXTRACT_AND_RUN=1
export DEPLOY_GTK_VERSION=4
export PATH="$TOOLS:$PATH"
export VERSION="$(cd "$ROOT" && git describe --tags --always 2>/dev/null || echo dev)"
export LDAI_OUTPUT="$OUT/Vault-x86_64.AppImage"

cd "$OUT"
"$TOOLS/linuxdeploy-x86_64.AppImage" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/vault" \
    --desktop-file "$ROOT/packaging/$APP_ID.desktop" \
    --icon-file "$ROOT/packaging/$APP_ID.svg" \
    --plugin gtk \
    --output appimage

echo
echo "Listo: $LDAI_OUTPUT"
