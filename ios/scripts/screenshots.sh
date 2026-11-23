#!/bin/sh
# Run the UI tests on the iPhone 17 simulator and export their screenshots, shrunk, to ios/docs/.
# Usage: ios/scripts/screenshots.sh   (on a Mac with Xcode 27 and xcodegen)
set -eu
cd "$(dirname "$0")/.."
RESULT="build/ui-tests.xcresult"
OUT="build/attachments"
rm -rf "$RESULT" "$OUT"
xcodegen generate --quiet
mkdir -p Silas.xcodeproj/project.xcworkspace/xcshareddata/swiftpm
cp Package.resolved Silas.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
xcodebuild test -project Silas.xcodeproj -scheme Silas \
  -destination "platform=iOS Simulator,name=iPhone 17" \
  -derivedDataPath build -resultBundlePath "$RESULT" -quiet
xcrun xcresulttool export attachments --path "$RESULT" --output-path "$OUT"
mkdir -p docs
# The manifest maps each exported file to the name the test gave it.
python3 - "$OUT" <<'PY'
import json, os, subprocess, sys
out = sys.argv[1]
for test in json.load(open(os.path.join(out, "manifest.json"))):
    for a in test.get("attachments", []):
        name = a.get("suggestedHumanReadableName", "").split("_")[0]
        if name in ("launch", "ready", "keyboard", "keyboard-settings"):
            dst = f"docs/{name}.png"
            subprocess.run(["sips", "-Z", "900", os.path.join(out, a["exportedFileName"]), "--out", dst], check=True, capture_output=True)
            print(dst)
PY
