#!/bin/sh
# Arma dist/AppDir para el AppImage (o tarball de prueba).
# Uso: ./installer/linux/stage-appimage.sh
# Requiere: Rust estable + Java 17+ (build.rs descarga LT solo la primera vez).
set -eu
ROOT=$(dirname "$(dirname "$(dirname "$(readlink -f "$0")")")")
cd "$ROOT"

cargo build --release
APPDIR="$ROOT/dist/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib" "$APPDIR/usr/share/lexpdf"

cp target/release/lexpdf "$APPDIR/usr/bin/"
cp assets/pdfium/libpdfium.so "$APPDIR/usr/lib/"
cp installer/linux/lexpdf.png "$APPDIR/"
cp installer/linux/lexpdf.desktop "$APPDIR/"
if [ -d assets/lt ]; then
  cp -r assets/lt "$APPDIR/usr/share/lexpdf/lt"
else
  echo 'AVISO: sin assets/lt (¿falló la descarga en build?). El AppImage pedirá lt/ igualmente.' >&2
fi

# JRE minimizado con jlink (~40-60MB). Usa el JDK del sistema o $JAVA_HOME.
if [ ! -x "$APPDIR/usr/share/lexpdf/jre/bin/java" ]; then
  if command -v jlink >/dev/null 2>&1; then
    JLINK=jlink
  elif [ -n "${JAVA_HOME:-}" ] && [ -x "$JAVA_HOME/bin/jlink" ]; then
    JLINK="$JAVA_HOME/bin/jlink"
  else
    echo 'AVISO: sin jlink (instala un JDK 17+ o define JAVA_HOME). AppImage sin JRE.' >&2
    JLINK=""
  fi
  if [ -n "$JLINK" ]; then
    "$JLINK" --compress=2 --strip-debug --no-header-files --no-man-pages \
      --add-modules java.base,java.logging,java.xml,java.naming,java.management,java.sql,java.desktop,java.net.http,jdk.httpserver,jdk.unsupported \
      --output "$APPDIR/usr/share/lexpdf/jre"
  fi
fi

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$HERE/usr/lib:${LD_LIBRARY_PATH:-}"
export PDFIUM_DYNAMIC_LIB_PATH="$HERE/usr/lib"
export LT_HOME="$HERE/usr/share/lexpdf/lt"
export JAVA_BIN="$HERE/usr/share/lexpdf/jre/bin/java"
exec "$HERE/usr/bin/lexpdf" "$@"
EOF
chmod +x "$APPDIR/AppRun"
"$APPDIR/usr/share/lexpdf/jre/bin/java" -version 2>&1 | head -n 1 || true
echo "OK: $APPDIR listo (AppRun + desktop + icono)"
