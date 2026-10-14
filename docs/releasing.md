# Releasing BoBe

BoBe releases are built and published from a maintainer Mac. Release automation lives under `scripts/tasks/`.

## Artifact chain

1. Build the Rust daemon, pinned Node/pi distribution, and Swift application.
   The build scripts reject BoBe's macOS Mach-O executables whose
   `LC_BUILD_VERSION` minimum is not 26.0. Upstream Node retains its own
   compatible deployment floor.
2. Assemble `build/BoBe.app`.
3. Sign the agent's native modules, Node, the embedded daemon, and Sparkle
   helpers/framework inside-out, then the app bundle.
4. Build and sign `build/BoBe-X.Y.Z.dmg`.
5. Notarize, staple, and validate the app and DMG.
6. Create and sign `build/BoBe-X.Y.Z.zip` for Sparkle.
7. Generate an appcast from staged archives.
8. Publish the archive before the appcast that references it.
9. Build standalone daemon archives on each target host. Generate and sign their
   canonical manifests on the release Mac with the same Ed25519 identity.

`SUPublicEDKey` and `SUFeedURL` live in `BoBeMacUI/BoBe/Resources/Info.plist`. The configured feed is:

```text
https://bobebot.com/updates/macos/appcast.xml
```

## Preflight

Use a clean, reviewed `main` checkout with no unrelated changes:

```bash
git status --short
git tag -s vX.Y.Z -m "BoBe X.Y.Z"
```

`scripts/tasks/ship.sh` fails unless the worktree is clean and `HEAD` is exactly the commit
named by the local `vX.Y.Z` tag. Push that tag before creating the GitHub
Release.

Confirm that the Developer ID identity is available and that the Apple Developer
team has issued a Developer ID provisioning profile for `com.bobe.app`. Apple
grants its keychain access as the team wildcard (`TEAMID.*`), which covers the
endpoint credential group. Supply its external build settings for every app
release:

```bash
security find-identity -v -p codesigning
export BOBE_DEVELOPMENT_TEAM="TEAMID1234"
export BOBE_PROVISIONING_PROFILE_SPECIFIER="BoBe Developer ID"
```

The release build fails before launch if these settings, the embedded profile, or
the profile-expanded Data Protection Keychain entitlements are missing or
inconsistent.

Release secrets and keys must remain outside the repository. Do not commit certificates, notarization credentials, Sparkle private keys, keychain exports, or deployment credentials.

## Bundled agent runtime

Node is pinned to 22.22.0. The SHA256 values in
`scripts/lib/agent-runtime.sh` come from the official
[release checksum list](https://nodejs.org/dist/v22.22.0/SHASUMS256.txt).
The build verifies the archive for each supported macOS/Linux ARM64/x64 host
before extraction. `BoBeAgent/package-lock.json` pins the production graph,
including pi 1.0.3. Never replace the archive digests with unverified values.

The app ships `Contents/MacOS/node` and
`Contents/Resources/agent/{dist/main.js,assets/,bin/,package.json,node_modules/}`;
the daemon is `Contents/MacOS/bobe-daemon`. JavaScript and package data belong
in Resources, not in a code-only directory; see Apple's
[code-signing bundle rules](https://developer.apple.com/library/archive/technotes/tn2206/_index.html).
Standalone archives put `node` and `agent/` beside `bin/bobe`.
The agent directory includes the Node license and upstream package
licenses, provider catalogs, templates, themes, and WebAssembly assets. Its
private `bin/` includes ripgrep 15.1.0 and fd 10.3.0, verified against pinned
upstream release archive digests. Search never depends on a user's global
tools or unpinned first-use downloads.
`scripts/tasks/notices.sh` writes BoBe's license, `TRADEMARKS.md`, the Rust crate
notices from `cargo-about`, the speech and app license files in `third_party/` and,
for the app, each Swift package's license into `Contents/Resources/licenses/`, or
`bin/licenses/` in a standalone archive.
Worker regression tests run before staging; test executables are not shipped.
Production npm executable links are not generated; signed standalone archives
contain only regular files and directories.

`scripts/check-agent-runtime.sh` checks the Node and pi versions, imports the
real SDK, reads its model catalog and assets, and compiles its WebAssembly
modules. Run it on the exact staged runtime. It is an artifact check, not proof
of provider authentication or a running companion session.

Node uses `BoBeMacUI/node.entitlements.plist` for V8 JIT and WebAssembly
executable memory. Native modules are signed before Node. Neither receives the
GUI's endpoint Keychain group.

## Release with Apple ID notarization

Store the app-specific password in the macOS Keychain once. `notarytool`
prompts for it without placing it in shell history or a process argument:

```bash
xcrun notarytool store-credentials "bobe-notary" \
  --apple-id "maintainer@example.com" \
  --team-id "TEAMID1234"
```

`scripts/tasks/ship.sh` runs clean, dependency resolution, build, signing, DMG creation, notarization, stapling, validation, and Sparkle ZIP creation:

```bash
export NOTARIZE_KEYCHAIN_PROFILE="bobe-notary"
scripts/tasks/ship.sh X.Y.Z "Developer ID Application: NAME (TEAMID1234)"
```

Never pass the app-specific password to `notarytool --password`; command
arguments are visible to other local processes.

## Release with an App Store Connect API key

Run the stages explicitly:

```bash
scripts/tasks/clean.sh
scripts/tasks/release.sh X.Y.Z "Developer ID Application: NAME (TEAMID1234)"
scripts/tasks/notarize.sh X.Y.Z /path/to/AuthKey.p8 KEY_ID ISSUER_ID
scripts/tasks/staple.sh X.Y.Z
scripts/tasks/sparkle-zip.sh X.Y.Z
```

## Sign the Sparkle archive

Sparkle's `sign_update` tool is resolved through SwiftPM:

```bash
scripts/tasks/sparkle-sign-update.sh X.Y.Z /path/to/sparkle-private-key
```

Save the emitted enclosure attributes for the generated appcast.

## Sign a standalone daemon update

Build each standalone archive on its target platform so the daemon, Node, and
production agent dependencies match that operating system and architecture.
Sparkle's `sign_update` runs only on macOS, so the release Mac generates and
signs the canonical manifest consumed by `bobe update apply`.

On a Linux target, build the archive, then copy it to the release Mac:

```bash
scripts/tasks/standalone.sh X.Y.Z
```

On the release Mac, sign each copied archive:

```bash
scripts/tasks/standalone-sign-update.sh \
  build/bobe-daemon-X.Y.Z-<platform>.tar.gz /path/to/sparkle-private-key
```

For the Mac's own platform, one command builds and signs:

```bash
scripts/tasks/standalone-release.sh X.Y.Z /path/to/sparkle-private-key
```

The build writes the archive and its checksum. Signing writes these files next
to the archive:

```text
build/bobe-daemon-X.Y.Z-<platform>.tar.gz.manifest.json
build/bobe-daemon-X.Y.Z-<platform>.tar.gz.manifest.json.sig
```

The manifest generator reads `compatibility.json` from the archive, validates
its platform, database schema version and attach-protocol range, hashes the
exact archive bytes, and records the archive basename. The signature file is the raw base64 Ed25519
signature emitted by Sparkle's pinned `sign_update -p` tool. Do not edit the
manifest after signing.

On the standalone host, apply the three files together with `bobe update apply`;
[Updating](cli.md#updating) shows the command.

The daemon pins the public key from `SUPublicEDKey`, verifies the signature and
archive hash, requires the live database to be at the candidate's schema
version and the attach-protocol ranges to overlap, creates
and verifies a backup, and rolls back if the candidate does not become ready.

## Generate the appcast

Stage only archives intended for publication:

```bash
rm -rf build/sparkle
mkdir -p build/sparkle
cp "build/BoBe-X.Y.Z.zip" build/sparkle/

scripts/tasks/sparkle-generate-appcast.sh \
  build/sparkle \
  https://bobebot.com/updates/macos/ \
  /path/to/sparkle-private-key \
  https://www.bobebot.com
```

Inspect `build/sparkle/appcast.xml` before publication. Confirm the version, download URL, file length, EdDSA signature, and `sparkle:minimumSystemVersion`.

BoBe releases must declare:

```xml
<sparkle:minimumSystemVersion>26.0</sparkle:minimumSystemVersion>
```

The pinned Sparkle generator infers this value from the staged app's
`LSMinimumSystemVersion`; `scripts/tasks/sparkle-generate-appcast.sh` verifies the newly
staged `CFBundleVersion` after generation. Do not edit the generated item by
hand because doing so invalidates Sparkle's signed metadata. Preserve older
appcast branch-point entries so pre-26 clients remain on their last compatible
release.

## Verify the exact payload

The source and assembled-app privacy manifests must match the audited contract:
UserDefaults with reason `CA92.1` is the only Required Reason API declaration;
tracking is disabled; and only user content, optional screen imagery, and the
authenticated provider account identifier are declared for cloud/remote flows.
Apple Speech assets and opt-in local MetricKit summaries stay on the Mac and do
not add collected-data declarations.

```bash
./scripts/check-privacy-manifest.sh
./scripts/check-privacy-manifest.sh \
  build/BoBe.app/Contents/Resources/PrivacyInfo.xcprivacy
```

Then verify signatures, Gatekeeper, and stapled tickets against the exact
payload that will be published:

```bash
./scripts/check-agent-runtime.sh \
  build/BoBe.app/Contents/MacOS build/BoBe.app/Contents/Resources/agent
codesign --verify --strict --verbose=2 \
  build/BoBe.app/Contents/MacOS/node
codesign --verify --deep --strict --verbose=2 build/BoBe.app
spctl --assess --type execute --verbose=4 build/BoBe.app
xcrun stapler validate build/BoBe.app
xcrun stapler validate "build/BoBe-X.Y.Z.dmg"

VERIFY_DIR="build/sparkle-verify-$$"
mkdir -m 0700 "$VERIFY_DIR"
trap 'rm -rf "$VERIFY_DIR"' EXIT
ditto -x -k "build/BoBe-X.Y.Z.zip" "$VERIFY_DIR"
./scripts/check-agent-runtime.sh \
  "$VERIFY_DIR/BoBe.app/Contents/MacOS" "$VERIFY_DIR/BoBe.app/Contents/Resources/agent"
codesign --verify --strict --verbose=2 \
  "$VERIFY_DIR/BoBe.app/Contents/MacOS/node"
codesign --verify --deep --strict --verbose=2 "$VERIFY_DIR/BoBe.app"
xcrun stapler validate "$VERIFY_DIR/BoBe.app"
```

## Publish safely

Publish the notarized DMG to the GitHub Release that the README sends users to.
For a new, already-pushed version tag:

```bash
gh release create "vX.Y.Z" "build/BoBe-X.Y.Z.dmg" \
  --verify-tag \
  --title "BoBe X.Y.Z" \
  --generate-notes
```

If the release already exists, use `gh release upload "vX.Y.Z"
"build/BoBe-X.Y.Z.dmg" --clobber` instead. Confirm the DMG is downloadable
from the public Releases page.

Upload the Sparkle ZIP next. Only then replace the public appcast. `HOST` and
`REMOTE_DIR` name the server and the directory that serve the update URL:

```bash
rsync -av "build/BoBe-X.Y.Z.zip" "$HOST:$REMOTE_DIR/"
rsync -av build/sparkle/appcast.xml "$HOST:$REMOTE_DIR/appcast.xml.next"
ssh "$HOST" "mv '$REMOTE_DIR/appcast.xml.next' '$REMOTE_DIR/appcast.xml'"
```

Verify with bounded requests:

```bash
curl --fail --location --connect-timeout 5 --max-time 20 \
  https://bobebot.com/updates/macos/appcast.xml
curl --fail --location --head --connect-timeout 5 --max-time 20 \
  "https://bobebot.com/updates/macos/BoBe-X.Y.Z.zip"
gh release view "vX.Y.Z" --json assets \
  --jq '.assets[] | select(.name == "BoBe-X.Y.Z.dmg") | .url'
```

If signing, notarization, staple validation, ZIP verification, or upload fails, stop. Keep the previous appcast live until the complete replacement archive is reachable.

## Troubleshooting

- **Check for Updates is disabled:** verify `SUFeedURL` is nonempty HTTPS.
- **Update rejected:** sign the exact ZIP being published and copy its emitted attributes into the appcast.
- **No update appears:** confirm the public appcast version is newer than the installed version and its archive URL is reachable.
- **Notarization fails:** inspect the `notarytool` log before retrying; do not staple an unaccepted artifact.
