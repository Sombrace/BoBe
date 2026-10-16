# Releasing

Releases are built, signed and published from a maintainer Mac with the
scripts in `scripts/tasks/`. Certificates, notarization credentials and the
Sparkle key stay outside the repository.

## Prepare

Bump the version in `BoBeService/Cargo.toml`, `PairingCore/Cargo.toml`,
`PairingFFI/Cargo.toml` (and their `=X.Y.Z` pins), `BoBeAgent/package.json`
and both apps' `Info.plist`, and refresh their lock files.
`scripts/check-release-version.sh X.Y.Z` lists any declaration that differs;
`ship.sh` and `standalone.sh` run it too. Merge into `main`, then tag.
`ship.sh` builds only from a clean checkout whose `HEAD` is the tag.

```bash
git tag -a vX.Y.Z -m "BoBe X.Y.Z" && git push origin vX.Y.Z
export BOBE_DEVELOPMENT_TEAM=TEAMID
export BOBE_PROVISIONING_PROFILE_SPECIFIER="BoBe Developer ID"
xcrun notarytool store-credentials PROFILE --apple-id APPLE_ID --team-id TEAMID
export NOTARIZE_KEYCHAIN_PROFILE=PROFILE
```

The provisioning profile is a Developer ID profile for `com.bobe.app`. The
scripts need Python 3.10 or later as `python3`. Never pass a password to
`notarytool --password`.

## Build

```bash
scripts/tasks/ship.sh X.Y.Z "Developer ID Application: NAME (TEAMID)"
```

`ship.sh` builds, signs, creates the DMG, notarizes, staples and writes the
Sparkle ZIP. Build each Linux archive on Ubuntu 24.04 (glibc 2.39) with
`scripts/tasks/standalone.sh X.Y.Z`, then copy it and its `.sha256` to
`build/` on the release Mac.

## Sign

`KEY` is a private file holding the Sparkle EdDSA key; delete it when done.

```bash
scripts/tasks/standalone-sign-update.sh build/bobe-daemon-X.Y.Z-linux-x86_64.tar.gz KEY
mkdir -p build/sparkle && cp build/BoBe-X.Y.Z.zip build/sparkle/
curl -fsSL https://bobebot.com/updates/macos/appcast.xml -o build/sparkle/appcast.xml
scripts/tasks/sparkle-generate-appcast.sh build/sparkle \
  https://github.com/Sombrace/BoBe/releases/download/vX.Y.Z/ KEY https://www.bobebot.com
```

Signing refuses an archive whose `.sha256`, name, version or binaries do not
match. The live feed beside the ZIP keeps the earlier items. Check the new item's
version, URL, length, signature and `minimumSystemVersion` 26.0, and never
edit a generated item.

## Verify

```bash
scripts/check-privacy-manifest.sh build/BoBe.app/Contents/Resources/PrivacyInfo.xcprivacy
scripts/check-agent-runtime.sh build/BoBe.app/Contents/MacOS build/BoBe.app/Contents/Resources/agent
codesign --verify --deep --strict build/BoBe.app
spctl --assess --type execute build/BoBe.app
xcrun stapler validate build/BoBe.app
xcrun stapler validate build/BoBe-X.Y.Z.dmg
```

Repeat the agent, `codesign` and `stapler` checks on the app inside the ZIP
(`ditto -x -k`).

## Publish

```bash
gh release create vX.Y.Z build/BoBe-X.Y.Z.dmg build/BoBe-X.Y.Z.zip \
  build/bobe-daemon-X.Y.Z-linux-x86_64.tar.gz* \
  --verify-tag --title "BoBe X.Y.Z" --notes-file NOTES --generate-notes
```

Then copy `build/sparkle/appcast.xml` to the website's
`public/updates/macos/appcast.xml`, point its download button at the new DMG
and publish the website. Macs are offered the update only after that.
