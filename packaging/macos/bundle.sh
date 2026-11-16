#!/usr/bin/env bash
# Builds a native macOS app bundle with the existing SVG icon and a local signature.
set -euo pipefail
trap 'echo "macOS app packaging failed at line $LINENO" >&2' ERR

if [[ "$(uname -s)" != Darwin ]]; then
    echo "macOS app packaging requires macOS (sips, iconutil, plutil, codesign)" >&2
    exit 1
fi

root="$(cd "$(dirname "$0")/../.." && pwd)"
# Select the repository's Rust toolchain even when invoked from another directory.
cd "$root"
target="$root/target"
bundle="$target/release/rtools.app"

echo "Building rtools in release mode"
cargo build --release --locked --package rtools --target-dir "$target"
package_id="$(cargo pkgid --package rtools)"
version="${package_id##*[@#]}"

work="$(mktemp -d "${TMPDIR:-/tmp}/rtools-bundle.XXXXXX")"
trap 'rm -rf "$work"' EXIT
iconset="$work/rtools.iconset"
mkdir -p "$iconset"

echo "Rendering application icons"
# Render each size from the vector source to keep Retina representations sharp.
for size in 16 32 128 256 512; do
    sips -s format png -z "$size" "$size" "$root/assets/icons/rtools.svg" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -s format png -z "$retina" "$retina" "$root/assets/icons/rtools.svg" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$work/rtools.icns"

cat > "$work/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>dev.rtools.rtools</string>
    <key>CFBundleName</key>
    <string>rtools</string>
    <key>CFBundleDisplayName</key>
    <string>rtools</string>
    <key>CFBundleExecutable</key>
    <string>rtools</string>
    <key>CFBundleIconFile</key>
    <string>rtools.icns</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>${version%%[-+]*}</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF
plutil -lint "$work/Info.plist"

echo "Assembling $bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m755 "$target/release/rtools" "$bundle/Contents/MacOS/rtools"
install -m644 "$work/Info.plist" "$bundle/Contents/Info.plist"
install -m644 "$work/rtools.icns" "$bundle/Contents/Resources/rtools.icns"
# Distribute attribution with the presets embedded in the executable.
mkdir -p "$bundle/Contents/Resources/licenses"
install -m644 "$root/assets/themes/LICENSE-APACHE" \
    "$bundle/Contents/Resources/licenses/gpui-kit-themes.txt"
install -m644 "$root/assets/themes/README.md" \
    "$bundle/Contents/Resources/licenses/gpui-kit-themes-source.md"

# A stable certificate keeps TCC identity across updates; ad-hoc access is build-specific.
codesign --force --sign "${CODESIGN_IDENTITY:--}" --timestamp=none "$bundle"
codesign --verify --strict "$bundle"
echo "Created $bundle"
if [[ "${CODESIGN_IDENTITY:--}" == - ]]; then
    echo "Ad-hoc build: after replacing an authorized app, remove and re-add rtools in Input Monitoring, enable it, and restart."
fi
