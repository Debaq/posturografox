#!/usr/bin/env bash
# Arma un AppImage con el binario de posturografox.
#
# Por qué AppImage y no .deb/.rpm: el release tiene que servir en el equipo que
# haya en el box, y ahí no se elige la distribución. Un binario suelto compilado
# en Ubuntu falla en una distro más vieja con "GLIBC_2.xx not found", que es
# justo el caso de un PC de hospital que nadie actualiza. El AppImage lleva
# adentro lo que hace falta y corre sin instalar nada.
#
# Todo el arte y el audio del juego están embebidos en el binario
# (`include_bytes!`), así que acá no hay que empaquetar ningún recurso: solo el
# ejecutable, su lanzador, el .desktop y el ícono.
#
# Requisitos: linuxdeploy en el PATH, o su ruta en $LINUXDEPLOY.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
RAIZ="$PWD"
APPDIR="$RAIZ/packaging/linux/AppDir"
SALIDA="$RAIZ/out"
VERSION="$(grep '^version' rust/Cargo.toml | head -1 | cut -d'"' -f2)"

echo "==> Compilando posturografox $VERSION"
cargo build --release --manifest-path rust/Cargo.toml
# El directorio de compilación puede estar movido por CARGO_TARGET_DIR o por la
# configuración del usuario, así que se le pregunta a cargo en vez de suponer
# "rust/target".
DESTINO="$(cargo metadata --format-version 1 --no-deps --manifest-path rust/Cargo.toml \
           | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')"
BIN="$DESTINO/release/posturografox"
[[ -x "$BIN" ]] || { echo "ERROR: no encontré el binario en $BIN" >&2; exit 1; }

echo "==> Armando AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$BIN" "$APPDIR/usr/bin/posturografox"
cp "$RAIZ/packaging/linux/posturografox.png" \
   "$APPDIR/usr/share/icons/hicolor/256x256/apps/posturografox.png"
cp "$RAIZ/packaging/linux/posturografox.png" "$APPDIR/posturografox.png"

cat > "$APPDIR/usr/share/applications/posturografox.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Posturografox
Comment=Posturografía de 4 celdas: centro de presión, CTSIB y rehabilitación
Exec=posturografox
Icon=posturografox
Categories=Science;MedicalSoftware;
Terminal=false
DESKTOP
cp "$APPDIR/usr/share/applications/posturografox.desktop" "$APPDIR/posturografox.desktop"

# El AppImage es de solo lectura, así que el programa se ejecuta desde la
# carpeta de datos del usuario: ahí caen el historial local, los CSV y los
# informes. La base de pacientes tiene su propia ruta y no depende de esto.
cat > "$APPDIR/AppRun" <<'APPRUN'
#!/usr/bin/env bash
AQUI="$(dirname "$(readlink -f "${0}")")"
export LD_LIBRARY_PATH="$AQUI/usr/lib:${LD_LIBRARY_PATH:-}"
DATOS="${XDG_DATA_HOME:-$HOME/.local/share}/posturografox"
mkdir -p "$DATOS"
cd "$DATOS"
exec "$AQUI/usr/bin/posturografox" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

echo "==> Empaquetando"
LD_BIN="${LINUXDEPLOY:-$(command -v linuxdeploy || true)}"
if [[ -z "$LD_BIN" || ! -x "$LD_BIN" ]]; then
    echo "ERROR: linuxdeploy no está. Bajalo de https://github.com/linuxdeploy/linuxdeploy" >&2
    echo "       o pasá su ruta en LINUXDEPLOY=. El AppDir quedó armado en $APPDIR" >&2
    exit 1
fi
mkdir -p "$SALIDA"
# NO_STRIP: el `strip` que trae linuxdeploy es más viejo que las bibliotecas de
# Mesa actuales y falla con "unknown type [0x13] section `.relr.dyn'". Lo único
# que se pierde es tamaño de paquete.
#
# El nombre del archivo lo decide linuxdeploy a partir del .desktop; se lo
# renombra después para que lleve la versión, que es lo que hace falta en un
# release donde conviven varias.
( cd "$SALIDA" && NO_STRIP=1 VERSION="$VERSION" "$LD_BIN" --appdir "$APPDIR" --output appimage )
APPIMAGE="$(ls -1t "$SALIDA"/*.AppImage 2>/dev/null | head -1)"
[[ -n "$APPIMAGE" ]] || { echo "ERROR: linuxdeploy no dejó ningún AppImage en $SALIDA" >&2; exit 1; }
FINAL="$SALIDA/Posturografox-$VERSION-x86_64.AppImage"
[[ "$APPIMAGE" == "$FINAL" ]] || mv "$APPIMAGE" "$FINAL"
chmod +x "$FINAL"
echo "Listo: $FINAL"
