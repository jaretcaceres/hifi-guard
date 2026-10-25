#!/bin/bash
# Builds "HiFi Guard.app". Pass --install to replace /Applications/HiFi Guard.app and launch it.
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
APP="target/HiFi Guard.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/hifiguard "$APP/Contents/MacOS/"
cp assets/Info.plist "$APP/Contents/"
cp assets/AppIcon.icns "$APP/Contents/Resources/"
codesign --force --sign - "$APP"
echo "Built $APP"

if [[ "${1:-}" == "--install" ]]; then
  pkill -x hifiguard 2>/dev/null || true
  pkill -x HiFiGuard 2>/dev/null || true
  rm -rf "/Applications/HiFi Guard.app"
  cp -R "$APP" /Applications/
  open "/Applications/HiFi Guard.app"
  echo "Installed and launched /Applications/HiFi Guard.app"
fi
