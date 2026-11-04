#!/bin/bash
# Builds "HiFi Guard.app".
#   --universal  builds for Apple silicon and Intel
#   --install    replaces /Applications/HiFi Guard.app and launches it
# Set HIFIGUARD_SIGN_IDENTITY to a Developer ID to sign for distribution (ad-hoc otherwise).
set -euo pipefail
cd "$(dirname "$0")"

UNIVERSAL=false
INSTALL=false
for arg in "$@"; do
  case "$arg" in
    --universal) UNIVERSAL=true ;;
    --install) INSTALL=true ;;
    *) echo "Unknown option: $arg" >&2; exit 1 ;;
  esac
done

if $UNIVERSAL; then
  cargo build --release --locked --target aarch64-apple-darwin
  cargo build --release --locked --target x86_64-apple-darwin
  BIN=target/universal/hifiguard
  mkdir -p target/universal
  lipo -create -output "$BIN" target/aarch64-apple-darwin/release/hifiguard target/x86_64-apple-darwin/release/hifiguard
else
  cargo build --release --locked
  BIN=target/release/hifiguard
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="target/HiFi Guard.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/hifiguard"
cp assets/Info.plist "$APP/Contents/"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" -c "Set :CFBundleVersion $VERSION" "$APP/Contents/Info.plist"
cp assets/AppIcon.icns "$APP/Contents/Resources/"

IDENTITY="${HIFIGUARD_SIGN_IDENTITY:-}"
if [[ -n "$IDENTITY" ]]; then
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"
else
  codesign --force --sign - "$APP"
fi
echo "Built $APP ($VERSION)"

if $INSTALL; then
  pkill -x hifiguard 2>/dev/null || true
  pkill -x HiFiGuard 2>/dev/null || true
  rm -rf "/Applications/HiFi Guard.app"
  cp -R "$APP" /Applications/
  open "/Applications/HiFi Guard.app"
  echo "Installed and launched /Applications/HiFi Guard.app"
fi
