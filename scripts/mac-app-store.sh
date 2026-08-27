#!/usr/bin/env bash
# Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the Mac App Store package. Tauri's bundler stops at the .app: it
# cannot embed a provisioning profile or produce the signed .pkg the Store
# requires, so the app is built unsigned here and the last three steps are
# done by hand.
#
# Upload the resulting .pkg with Transporter, or:
#   xcrun altool --upload-app -f <pkg> -t macos -u <apple-id> -p <app-specific-password>

set -euo pipefail

cd "$(dirname "$0")/.."

TEAM_ID="8Z8ZP4SX98"
BUNDLE_ID="com.skapacraft.nostos"
APP_CERT="Apple Distribution: Daniele Pio Gagliardi ($TEAM_ID)"
PKG_CERT="3rd Party Mac Developer Installer: Daniele Pio Gagliardi ($TEAM_ID)"
PROFILE="$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles/6eab081f-731f-40a3-ad1a-5f5baf1969c9.provisionprofile"
ENTITLEMENTS="src-tauri/macos/entitlements-appstore.plist"

APP="src-tauri/target/universal-apple-darwin/release/bundle/macos/Nostos.app"
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
PKG="Nostos-$VERSION.pkg"

[ -f "$PROFILE" ] || { echo "Provisioning profile not found: $PROFILE" >&2; exit 1; }

# Tauri would sign during the build if APPLE_SIGNING_IDENTITY were set, but the
# profile has to be inside the bundle before signing, so signing happens after.
echo "==> Building universal .app"
npm run tauri build -- --target universal-apple-darwin --bundles app

echo "==> Embedding provisioning profile"
cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"

# --deep is deprecated for signing but still the only way to reach the WebView
# helpers Tauri nests inside the bundle; they are signed before the outer app
# because signing an enclosing bundle seals whatever is already inside it.
echo "==> Signing"
codesign --force --deep --timestamp --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$APP_CERT" \
  "$APP"

codesign --verify --strict --verbose=2 "$APP"

echo "==> Building .pkg"
productbuild --component "$APP" /Applications --sign "$PKG_CERT" "$PKG"

echo
echo "Built $PKG"
echo "Upload it with Transporter, or with xcrun altool (see the header of this script)."
