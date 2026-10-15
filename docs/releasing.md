# Releasing

Releases are built, signed and published from a maintainer Mac with the
scripts in `scripts/tasks/`. Keep certificates, notarization credentials and
Sparkle keys out of the repository.

## Prepare

```bash
git status --short                       # must be clean, on reviewed main
git tag -s vX.Y.Z -m "BoBe X.Y.Z"         # ship.sh requires HEAD to be this tag
security find-identity -v -p codesigning
export BOBE_DEVELOPMENT_TEAM="TEAMID1234"
export BOBE_PROVISIONING_PROFILE_SPECIFIER="BoBe Developer ID"
xcrun notarytool store-credentials "bobe-notary" \
  --apple-id "maintainer@example.com" --team-id "TEAMID1234"
```

The team needs a Developer ID provisioning profile for `com.bobe.app`; its
`TEAMID.*` keychain group covers the endpoint credential group. The build
stops if the profile or its entitlements are missing or inconsistent.

Node, ripgrep and fd are pinned by SHA-256 in `scripts/lib/agent-runtime.sh`.
Never replace a digest with an unverified value.

## Build, sign and notarize

```bash
export NOTARIZE_KEYCHAIN_PROFILE="bobe-notary"
scripts/tasks/ship.sh X.Y.Z "Developer ID Application: NAME (TEAMID1234)"
```

`ship.sh` cleans, builds, signs inside-out, creates the DMG, notarizes,
staples, validates and writes the Sparkle ZIP. With an App Store Connect API
key, run the stages yourself:

```bash
scripts/tasks/clean.sh
scripts/tasks/release.sh X.Y.Z "Developer ID Application: NAME (TEAMID1234)"
scripts/tasks/notarize.sh X.Y.Z /path/to/AuthKey.p8 KEY_ID ISSUER_ID
scripts/tasks/staple.sh X.Y.Z
scripts/tasks/sparkle-zip.sh X.Y.Z
```

Never pass a password to `notarytool --password`; arguments are visible to
other processes.

## Standalone daemon archives

Build each archive on its target platform, then sign its manifest on the
release Mac:

```bash
scripts/tasks/standalone.sh X.Y.Z                                   # on the target
scripts/tasks/standalone-sign-update.sh \
  build/bobe-daemon-X.Y.Z-<platform>.tar.gz /path/to/sparkle-private-key
scripts/tasks/standalone-release.sh X.Y.Z /path/to/sparkle-private-key  # Mac platform: build and sign
```

Signing writes `<archive>.manifest.json` and `<archive>.manifest.json.sig`. Do
not edit the manifest after signing. Owners apply the three files with
`bobe update apply` ([cli.md](cli.md#update)).

## Sparkle appcast

```bash
scripts/tasks/sparkle-sign-update.sh X.Y.Z /path/to/sparkle-private-key
rm -rf build/sparkle && mkdir -p build/sparkle
cp "build/BoBe-X.Y.Z.zip" build/sparkle/
scripts/tasks/sparkle-generate-appcast.sh build/sparkle \
  https://bobebot.com/updates/macos/ /path/to/sparkle-private-key https://www.bobebot.com
```

Check `build/sparkle/appcast.xml` for the version, URL, length, signature and
`<sparkle:minimumSystemVersion>26.0</sparkle:minimumSystemVersion>`. Never edit
a generated item, and keep older branch-point entries. `SUFeedURL` and
`SUPublicEDKey` are in `BoBeMacUI/BoBe/Resources/Info.plist`.

## Verify the exact payload

```bash
./scripts/check-privacy-manifest.sh build/BoBe.app/Contents/Resources/PrivacyInfo.xcprivacy
./scripts/check-agent-runtime.sh build/BoBe.app/Contents/MacOS build/BoBe.app/Contents/Resources/agent
codesign --verify --deep --strict --verbose=2 build/BoBe.app
spctl --assess --type execute --verbose=4 build/BoBe.app
xcrun stapler validate build/BoBe.app
xcrun stapler validate "build/BoBe-X.Y.Z.dmg"
```

Repeat the agent, `codesign` and `stapler` checks on the app extracted from the
Sparkle ZIP (`ditto -x -k`).

## Publish

Publish in this order and stop at the first failure; the previous appcast
stays live until its replacement archive is reachable.

```bash
gh release create "vX.Y.Z" "build/BoBe-X.Y.Z.dmg" --verify-tag --title "BoBe X.Y.Z" --generate-notes
rsync -av "build/BoBe-X.Y.Z.zip" "$HOST:$REMOTE_DIR/"
rsync -av build/sparkle/appcast.xml "$HOST:$REMOTE_DIR/appcast.xml.next"
ssh "$HOST" "mv '$REMOTE_DIR/appcast.xml.next' '$REMOTE_DIR/appcast.xml'"
curl --fail --location --max-time 20 https://bobebot.com/updates/macos/appcast.xml
curl --fail --location --head --max-time 20 "https://bobebot.com/updates/macos/BoBe-X.Y.Z.zip"
```
