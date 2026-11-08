#!/bin/bash
set -euo pipefail

if [[ "${GITHUB_ACTIONS:-}" != true || "${RUNNER_ENVIRONMENT:-}" != github-hosted || "${RUNNER_OS:-}" != macOS || "$(uname -m)" != arm64 ]]; then
  echo 'Use this signing script on a GitHub-hosted macOS arm64 runner.' >&2
  exit 1
fi
: "${CODESIGN_CERTIFICATE_P12_BASE64:?Set this secret in the codesign environment}"
: "${CODESIGN_CERTIFICATE_PASSWORD:?Set this secret in the codesign environment}"

certificate_sha256=FEEE897C91F36CAFE6AE34CD4AD50F701C89D688DF68DD22D09E0FFD77635FEB
app_name='Orivelle Orbit.app'
app="$GITHUB_WORKSPACE/runtime/target/aarch64-apple-darwin/release/bundle/macos/$app_name"
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)*$ ]]; then
  echo 'The application bundle has an invalid version.' >&2
  exit 1
fi
output="$GITHUB_WORKSPACE/runtime/candidates/Orivelle-Orbit-$version-macos-arm64-self-signed.dmg"

umask 077
work="$(mktemp -d "$RUNNER_TEMP/orbit-self-sign.XXXXXX")"
keychain="$work/signing.keychain-db"
mount="$work/mounted"
mounted=false
cleanup() {
  if [[ "$mounted" == true ]]; then
    /usr/bin/hdiutil detach "$mount" -force >/dev/null || true
  fi
  if [[ -f "$keychain" ]]; then
    /usr/bin/security delete-keychain "$keychain" >/dev/null || true
  fi
  rm -rf -- "$work"
}
trap cleanup EXIT

fingerprint() {
  /usr/bin/openssl x509 -noout -fingerprint "$@" | cut -d= -f2 | tr -d ':' | tr '[:lower:]' '[:upper:]'
}

keychains=("$keychain")
existing_keychains="$(/usr/bin/security list-keychains -d user)"
while IFS='"' read -r _ entry _; do
  if [[ -n "$entry" ]]; then keychains+=("$entry"); fi
done <<< "$existing_keychains"
keychain_password="$(/usr/bin/openssl rand -base64 32)"
printf '::add-mask::%s\n' "$keychain_password"
printf '%s' "$CODESIGN_CERTIFICATE_P12_BASE64" | /usr/bin/openssl base64 -d -A -out "$work/signing.p12"
/usr/bin/security create-keychain -p "$keychain_password" "$keychain"
/usr/bin/security set-keychain-settings -lut 3600 "$keychain"
/usr/bin/security unlock-keychain -p "$keychain_password" "$keychain"
/usr/bin/security import "$work/signing.p12" -k "$keychain" -t cert -f pkcs12 \
  -P "$CODESIGN_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
rm -- "$work/signing.p12"
unset CODESIGN_CERTIFICATE_P12_BASE64 CODESIGN_CERTIFICATE_PASSWORD
/usr/bin/security list-keychains -d user -s "${keychains[@]}"
/usr/bin/security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
unset keychain_password

/usr/bin/security find-certificate -c 'Morton Li' -p "$keychain" > "$work/leaf.pem"
if [[ "$(fingerprint -sha256 -in "$work/leaf.pem")" != "$certificate_sha256" ]]; then
  echo 'The imported certificate does not match the pinned Morton Li certificate.' >&2
  exit 1
fi
/usr/bin/openssl x509 -in "$work/leaf.pem" -checkend 0 -noout
identity="$(fingerprint -sha1 -in "$work/leaf.pem")"
if [[ "$(/usr/bin/security find-identity -p codesigning "$keychain")" != *"$identity"* ]]; then
  echo 'The signing certificate has no accessible private key.' >&2
  exit 1
fi

mkdir -m 755 "$work/image"
staged_app="$work/image/$app_name"
/usr/bin/ditto "$app" "$staged_app"
ln -s /Applications "$work/image/Applications"
/usr/bin/codesign --force --sign "$identity" --keychain "$keychain" --timestamp=none --options runtime "$staged_app"

image="$work/orbit.dmg"
/usr/bin/hdiutil create -volname 'Orivelle Orbit' -fs HFS+ -format UDZO -srcfolder "$work/image" "$image"
/usr/bin/codesign --sign "$identity" --keychain "$keychain" --timestamp=none "$image"
/usr/bin/codesign --verify --strict "$image"
/usr/bin/hdiutil verify "$image"
mkdir "$mount"
/usr/bin/hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$mount" "$image" >/dev/null
mounted=true
packaged_app="$mount/$app_name"
/usr/bin/codesign --verify --deep --strict -R 'identifier "io.orivelle.orbit"' "$packaged_app"
/usr/bin/codesign --display --extract-certificates="$work/embedded-" "$packaged_app"
if [[ "$(fingerprint -sha256 -inform DER -in "$work/embedded-0")" != "$certificate_sha256" ]]; then
  echo 'The packaged app uses a different certificate.' >&2
  exit 1
fi
executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$packaged_app/Contents/Info.plist")"
if [[ "$(/usr/bin/lipo -archs "$packaged_app/Contents/MacOS/$executable")" != arm64 ]]; then
  echo 'The packaged app is not macOS arm64.' >&2
  exit 1
fi
/usr/bin/hdiutil detach "$mount" >/dev/null
mounted=false
mv -- "$image" "$output"
printf 'Verified the self-signed macOS arm64 disk image with certificate %s.\n' "$certificate_sha256"
