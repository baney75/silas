#!/bin/sh
# Build Silas for the iPad, install it over USB and launch it.
# Usage: ios/scripts/install.sh <device-udid>   (or set SILAS_DEVICE)
# List devices with: xcrun devicectl list devices
set -eu
cd "$(dirname "$0")/.."
DEVICE="${1:-${SILAS_DEVICE:-}}"
if [ -z "$DEVICE" ]; then
  echo "usage: ios/scripts/install.sh <device-udid>   (or set SILAS_DEVICE)" >&2
  echo "find it with: xcrun devicectl list devices" >&2
  exit 2
fi

xcodegen generate --quiet
# Restore the committed pin: the generated project is ignored, so without this every
# install would resolve FluidAudio (and anything it pulls in) fresh.
mkdir -p Silas.xcodeproj/project.xcworkspace/xcshareddata/swiftpm
cp Package.resolved Silas.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
xcodebuild -project Silas.xcodeproj -scheme Silas -configuration Release \
  -destination "id=$DEVICE" -derivedDataPath build -allowProvisioningUpdates -disableAutomaticPackageResolution \
  DEVELOPMENT_TEAM=54H7JX2PD4 build -quiet
xcrun devicectl device install app --device "$DEVICE" build/Build/Products/Release-iphoneos/Silas.app
xcrun devicectl device process launch --device "$DEVICE" com.baney.silas.ipad
