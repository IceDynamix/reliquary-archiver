#!/bin/bash
set -e

APPIMAGE_NAME="$1"
APP_ID="reliquary-archiver"

if [ -z "$APPIMAGE_NAME" ]; then
  APPIMAGE_NAME="$APP_ID-x86_64.AppImage"
fi

echo -e "\$APPIMAGE_NAME: $APPIMAGE_NAME\n"

APP_DIR="AppDir"
USER_DIR="$APP_DIR/usr"
ICON_DEST="$USER_DIR/share/icons/hicolor/256x256/apps/$APP_ID.png"
DESKTOP_DEST="$USER_DIR/share/applications/$APP_ID.desktop"

mkdir -p "$USER_DIR/bin" \
  "$USER_DIR/lib" \
  "$(dirname $DESKTOP_DEST)" \
  "$(dirname $ICON_DEST)"

export APP_ID
envsubst '$APP_ID' <<'EOF' >"$APP_DIR/AppRun"
#!/bin/bash
set -e

HERE="$(dirname -- "$(readlink -f -- "$0")")"
export LD_LIBRARY_PATH="$HERE"/usr/lib
exec "$HERE/usr/bin/$APP_ID" "$@"
EOF
export -n APP_ID

chmod +x "$APP_DIR/AppRun"
cat "$APP_DIR/AppRun"
echo

cat >"$DESKTOP_DEST" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_ID
Exec=$APP_ID
Icon=$APP_ID
Terminal=true
Categories=Utility;
EOF

chmod +x "$DESKTOP_DEST"
cat "$DESKTOP_DEST"

BIN_SRC="target/release/$APP_ID"
chmod +x "$BIN_SRC"
cp "$BIN_SRC" "$USER_DIR/bin/"
echo

ICON_SRC="assets/icon256.png"
cp "$ICON_SRC" "$ICON_DEST"

ln -sr "$DESKTOP_DEST" "$APP_DIR/$(basename $DESKTOP_DEST)"
ln -sr "$ICON_DEST" "$APP_DIR/$(basename $ICON_DEST)"

LIBPCAP_PATH=$(ldconfig -p | grep "libpcap.so" | awk '{print $NF}' | head -n 1)
cp "$LIBPCAP_PATH" "$USER_DIR/lib/"

echo "--- list $APP_DIR ---"
ls -Rlh "$APP_DIR"
echo

APPIMAGE_TOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
APPIMAGE_TOOL="appimagetool.AppImage"

curl -L "$APPIMAGE_TOOL_URL" -o "$APPIMAGE_TOOL"
chmod +x "$APPIMAGE_TOOL"

if ! ./"$APPIMAGETOOL" "$APP_DIR" "$APPIMAGE_NAME"; then
  [ -d "./squashf-root" ] && rm -rf "./squashfs-root"

  echo "fuse mount failed. extracting.."
  ./"$APPIMAGE_TOOL" --appimage-extract
  ./squashfs-root/AppRun "$APP_DIR" "$APPIMAGE_NAME"
fi
