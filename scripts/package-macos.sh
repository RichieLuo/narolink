#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    printf '%s\n' 'Build the Naro macOS package on macOS.' >&2
    exit 1
fi

metadata=$(cargo metadata --locked --no-deps --format-version 1)
version=$(printf '%s' "$metadata" | plutil -extract packages.0.version raw -o - -- -)
target_dir=$(printf '%s' "$metadata" | plutil -extract target_directory raw -o - -- -)
cargo build --release --locked
executable="$target_dir/release/narolink"
architecture=$(lipo -archs "$executable")
architecture=$(printf '%s' "$architecture" | tr ' ' '-')
archive="dist/Naro-$version-macOS-$architecture.zip"

mkdir -p dist
staging=$(mktemp -d "$PWD/dist/.naro-package.XXXXXX")
trap 'rm -rf "$staging"' EXIT
app="$staging/Naro.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$executable" "$app/Contents/MacOS/narolink"
install -m 644 assets/narolink.icns "$app/Contents/Resources/Naro.icns"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key>
    <string>Naro</string>
    <key>CFBundleName</key>
    <string>Naro</string>
    <key>CFBundleIdentifier</key>
    <string>chat.bato.narolink</string>
    <key>CFBundleExecutable</key>
    <string>narolink</string>
    <key>CFBundleIconFile</key>
    <string>Naro.icns</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSEnvironment</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
    </dict>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

plutil -lint "$app/Contents/Info.plist"
signing_identity=${NAROLINK_MACOS_SIGN_IDENTITY:-$(git config --local --get narolink.macosSigningIdentity 2>/dev/null || true)}
if [ -n "$signing_identity" ] && [ "$signing_identity" != - ]; then
    codesign --force --sign "$signing_identity" "$app"
else
    codesign --force --sign - --timestamp=none "$app"
fi
codesign --verify --strict "$app"
ditto -c -k --norsrc --keepParent "$app" "$staging/Naro.zip"
mv -f "$staging/Naro.zip" "$archive"
printf '%s\n' "$archive"
