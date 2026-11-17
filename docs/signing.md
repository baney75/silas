# macOS signing and permissions

macOS stores Silas's Microphone and Accessibility grants (TCC) against the app's
**designated requirement**: the rule in its code signature that says which future builds count
as "the same app". The bundle ID `com.baney.silas` alone is not enough.

- **Ad-hoc** (`APPLE_SIGNING_IDENTITY=-`): the requirement is the build's own hash
  (`cdhash ...`). Every build is a new app to macOS. After an update the switch in
  Privacy & Security can still show on while the grant no longer applies, and Silas has to ask
  again.
- **A real certificate**: the requirement names the bundle ID and the certificate chain, so
  every build signed with the same certificate keeps the grants.

Check any build with:

```sh
codesign -d -r- /Applications/Silas.app
```

A line containing `cdhash` means ad-hoc. A stable one looks like
`identifier "com.baney.silas" and anchor apple generic and certificate leaf[subject.OU] = <TEAMID> ...`.
That line must stay the same from release to release, or every user is asked again.

## Stable channel

`scripts/release-stable.sh` signs on Donovan's Mac, from the `vX.Y.Z` tag `promote.yml` made
(docs/channels.md). It picks, in order:

1. `APPLE_SIGNING_IDENTITY` if set (an explicit override),
2. the first `Developer ID Application:` identity in the keychain,
3. Silas Local Signing (below), unless `SILAS_LOCAL_SIGNING=0`,
4. the first `Apple Development:` identity.

Developer ID Application is best when it exists: its requirement pins the Team ID, which
survives certificate renewal, and CI can use it too. Without it, Silas Local Signing keeps the
grants for every Mac-built release, with no Apple Developer membership needed. Any switch of
identity changes the requirement once, so users are asked once more after that release.

### Silas Local Signing

`scripts/signing-identity.sh` is the BaneySpaces Local Signing model applied to Silas. It keeps
a self-signed code-signing certificate named `Silas Local Signing` in its own keychain:

```text
~/Library/Application Support/Silas/Signing/
  silas-signing.keychain-db   the keychain
  keychain-password           its random password (0600)
  kept/identity.p12           the identity, so a lost keychain comes back with the same leaf
  kept/identity-password      the .p12 password (0600)
```

`signing-identity.sh ensure` creates the identity on first use, restores it from `kept/` if the
keychain is gone, unlocks it, adds it to the keychain search list, and prints its SHA-1 for
`codesign`. Passwords move through files and `security -i` on stdin, never a command's
arguments. `signing-identity.sh requirement` prints the requirement it should give:

```text
identifier "com.baney.silas" and certificate leaf = H"<sha1 of the certificate>"
```

The certificate never leaves the Mac. Back up `kept/` with the Mac: losing it means a new
certificate, a new requirement, and one more round of permission prompts.

**First release with Local Signing.** The pin still holds the v0.1.6 Apple Development
requirement, so the first release signed with Local Signing (or with Developer ID) stops at the
requirement check. Run it once as `ALLOW_NEW_REQUIREMENT=1 scripts/release-stable.sh ...`. That
rewrites `scripts/release-requirement.txt` to what `codesign` prints; land that change on `main`
in a pull request after the release, and every later Local Signing build must match it. Users grant Accessibility once more
after that release, then never again for Mac-built releases.

The script prints the new build's requirement, refuses an ad-hoc one, and stops if it differs
from the installed `/Applications/Silas.app` (or `~/Applications/Silas.app`). Set
`ALLOW_NEW_REQUIREMENT=1` for an intended change such as that one-time switch.

`scripts/release-requirement.txt` pins the requirement of the last stable release (v0.1.6,
Apple Development). `scripts/check-requirement.sh` compares every build to it: the release
script stops on a mismatch, a signed nightly fails before upload, and CI checks the pin itself.
Only a stable release with `ALLOW_NEW_REQUIREMENT=1` rewrites the pin, and a pull request to
`main` carries the new line. An ad-hoc installed copy holds no grants worth keeping, so the release
script skips it and goes by the pin. Once the Developer ID secrets exist, signed nightlies fail
until the first Developer ID stable release moves the pin.

## Nightly and dev channels

`.github/workflows/channel-build.yml` (called by `nightly.yml` and `dev.yml`) publishes a Mac
build only when it is signed with a real certificate from these three secrets. Without them it still builds the Mac app, to prove it
compiles, but uploads nothing for macOS: the nightly ships for Linux and Windows, `latest.json`
has no `darwin-aarch64` entry, and the release notes say so. Publish fails if a darwin asset
shows up without the secrets.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of a `.p12` export of a **Developer ID Application** certificate and its private key (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | the password set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | the identity's full name, e.g. `Developer ID Application: Donovan Baney (TEAMID)` |

CI cannot create a stable identity on its own, and it cannot use Silas Local Signing: that
private key stays on the Mac and never goes into repo secrets. Only a Developer ID Application
certificate in these secrets gives CI-built nightlies a requirement that holds from build to
build. Until the three secrets exist, there are no Mac nightlies. An ad-hoc nightly was worse
than none: each one is a new app to macOS, so Accessibility stays on in Settings for the old
signature while the new build is not trusted (2026-11-02, 0.1.7-nightly.202611021637).

A Mac on the nightly channel finds no update for its platform until then, so the update check
there reports an error instead of an update.

Use the same Developer ID certificate as the stable channel, so moving between channels keeps
the grants too.

## In-app updates

The updater in `src-tauri/src/updates.rs` checks the signature before it replaces anything.
On macOS it downloads the update, unpacks it to a scratch folder, and compares
`codesign -d -r-` of the running `Silas.app` with the downloaded one. It installs only when the
two designated requirements are the same string (`same_designated_requirement`, the same rule
as BaneyMacUpdater). Two different ad-hoc builds never match, because each requirement is its
own `cdhash`; the identical build does. An unreadable signature counts as different. A refused
update shows "This update is signed differently from your install. Installing it would break
Accessibility. Not installed." and auto-update skips that version until Silas restarts. The
updater never calls `tccutil`.

## Every way a Mac gets a new Silas.app

Each path refuses to put a build with a different designated requirement over a signed install,
so a user never has to touch Accessibility after an update.

| Path | How it fails closed |
| --- | --- |
| In-app updater (`src-tauri/src/updates.rs`) | Compares `codesign -d -r-` of the running bundle and the download, installs only on the same string, then runs `codesign --verify --deep --strict -R=<installed requirement>` on the download. Anything else is refused and not installed (above). |
| BaneyMacUpdater (BaneyFleet, outside this repo) | Same rule: refuses when the designated requirements differ. `same_designated_requirement` in `updates.rs` mirrors it. |
| `install.sh` on macOS | After the download and before Silas is quit or replaced, reads the requirement of the download and of every installed copy (`SILAS_INSTALL_DIR`, or `/Applications` and `~/Applications`). A signed install with a different requirement stops the install and leaves the old copy alone. A matching requirement counts only when `codesign --verify --deep --strict -R=<installed requirement>` passes on the download, as in the updater. An ad-hoc or unsigned download over a signed install is always refused. `SILAS_ALLOW_NEW_SIGNATURE=1` allows a planned certificate change, such as the move to Developer ID, and only when the download verifies against its own requirement. An installed copy whose signature exists but can't be read stops the install before Silas is quit. A missing, unsigned or ad-hoc install holds no grant that survives an update, so it is replaced. `scripts/install-requirement-test.sh` covers each case with a stand-in `codesign`. |
| `scripts/release-stable.sh` | Stops unless the build matches `scripts/release-requirement.txt` (`check-requirement.sh --update`) and any signed installed copy. Only `ALLOW_NEW_REQUIREMENT=1` moves the pin. The `.app.tar.gz` and the `.dmg` are both made from that checked `Silas.app`. |
| DMG drag install | Finder copies whatever is in the image and can't be guarded at install time. The image is built from the app `release-stable.sh` just checked, so its requirement matches the pin. |
| `.github/workflows/channel-build.yml` (nightly and dev) | Uploads no Mac asset unless the three signing secrets exist, and a signed nightly must pass `check-requirement.sh` against the pin before upload. Publish fails if a darwin asset shows up without the secrets. |
| Silas Local Signing (`scripts/signing-identity.sh`) | Keeps one self-signed identity, restored from `kept/` if the keychain is lost, so every Mac-built release has the same requirement. Moving to it, or away from it, is a one-time pin move with `ALLOW_NEW_REQUIREMENT=1`. |

No Makefile or `package.json` script places a `.app`; `bun run tauri build` only writes into
`src-tauri/target`. `install.ps1` is Windows only.

## A switch that looks on but isn't

TCC keeps a row per service and client, and the row stores the client's code requirement
(C.W.K., "TCC: What macOS Remembers About Your App"). When a build with a different
designated requirement replaces Silas, System Settings can still show Silas on under
Accessibility, because the row belongs to the old signature, while `AXIsProcessTrusted` says no
for the new process. Seen on rectory1, 2026-11-02.

Turning the switch off and on does not fix it: the row keeps the old requirement (confirmed by
Donovan, 2026-11-02). `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt` doesn't
clear it either: it returns the current answer, and the prompt it shows is asynchronous
([Apple](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)).
Removing the row does. So Silas never tells a user to toggle the switch.

When the key listener can't start, the home screen shows "Silas can't hear your key yet" with:

1. **Fix Permission** (`fix_accessibility`). It reads first and does nothing when Silas is
   trusted. Otherwise it runs `tccutil reset Accessibility com.baney.silas`, which clears only
   Silas's own row, and reopens Silas: a shell waits for this process to exit, then
   `open`s the same bundle with `--after-accessibility-fix`. The new process asks once with
   `AXIsProcessTrustedWithOptions` (prompt on) and opens Privacy & Security, Accessibility. A dev
   build with no bundle asks from the running process instead. Each step logs a
   `silas.permissions` line ("fix: ... reset, relaunching to ask once", "after fix: ...
   requesting once, opening Settings").
2. **Open Settings** and numbered steps for doing it by hand: select Silas, click minus to remove
   it, click plus and add Silas from Applications, turn on its switch.

The reset runs only on that click, never at launch. The home screen re-reads the permission
every second while the banner shows, and the key listener retries every two seconds, so the
banner clears by itself once access is granted. First-time setup keeps the onboarding path:
Allow asks once, then opens Settings, and "Reset it and ask again" clears the row there.

Accessibility is the gate for the key listener. Silas does not read Input Monitoring.

## Asking for permissions

Separately from signing, Silas asks as little as it can (`src-tauri/src/permissions.rs`):

- Launch, `get_state` and the onboarding poll only read (`AXIsProcessTrusted`,
  `authorizationStatusForMediaType`). They never show a prompt.
- A click on Allow reads first. If access is already granted, no system prompt is shown.
- Each system prompt is shown at most once per run. A second click while still denied opens
  the Settings pane instead.
- Fix Permission on the home screen and "Reset it and ask again" in onboarding are the only
  paths that clear Silas's TCC entry, each on a click and only when the read says Silas is not
  trusted.

Every read and every request or skip logs a `silas.permissions` line, so a run shows
"preflight accessibility=Granted; no request" without touching TCC.

## Developer ID setup

A Developer ID Application certificate gives both the stable release and CI nightlies one
designated requirement pinned to the Team ID. It needs a paid Apple Developer Program
membership.

1. **Enroll** at <https://developer.apple.com/programs/>. The Apple Developer Program is
   **US$99 a year** for an individual or an organization. The Enterprise Program ($299) is not
   needed. See <https://developer.apple.com/programs/enroll/>.
2. **Individual or organization.** An individual enrolls with an Apple Account and needs no
   D-U-N-S number. An organization must be a legal entity and needs a D-U-N-S number. D&B issues
   one free in most countries, in up to about five business days, and Apple can take about two
   more days to see it. See <https://developer.apple.com/help/account/membership/D-U-N-S/>.
3. **Team ID.** This project's team is `54H7JX2PD4`; its Apple Development certificate already
   signs v0.1.6.
4. **Create the certificate.** As Account Holder, create a **Developer ID Application**
   certificate in Certificates, Identifiers & Profiles, or in Xcode under Settings, Accounts,
   Manage Certificates. See <https://developer.apple.com/developer-id/>.
5. **Export a `.p12`.** In Keychain Access, select the certificate together with its private key
   and export them as `DeveloperID.p12`, with a password Donovan chooses. Never commit the
   `.p12` or print its password.
6. **GitHub secrets** on `baney75/silas`. The names must match `.github/workflows/channel-build.yml`:

   | Secret | Value |
   | --- | --- |
   | `APPLE_CERTIFICATE` | `base64 -i DeveloperID.p12 \| pbcopy`, then paste |
   | `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
   | `APPLE_SIGNING_IDENTITY` | the full name, e.g. `Developer ID Application: Donovan Baney (54H7JX2PD4)`. Confirm it with `security find-identity -v -p codesigning` after creating the certificate. |

7. **Notarization.** Gatekeeper warns on the first open of a Developer ID build that isn't
   notarized. TCC grants persist either way. Create an app-specific password at
   <https://appleid.apple.com> (Sign-In and Security, App-Specific Passwords), then add:

   | Secret | Value |
   | --- | --- |
   | `APPLE_ID` | the Apple Account email of the developer team member |
   | `APPLE_TEAM_ID` | `54H7JX2PD4` |
   | `APPLE_PASSWORD` | the app-specific password |

   `APPLE_PASSWORD` is the name the Tauri bundler reads: with `APPLE_ID`, `APPLE_PASSWORD` and
   `APPLE_TEAM_ID` in its environment it submits the app with `notarytool` and staples the
   ticket (<https://v2.tauri.app/distribute/sign/macos/>). **Nightly does not wire them today**:
   the Build step passes only the three signing secrets. Add the three lines to that step's
   `env` once the secrets exist, inside the signed branch so an unsigned build never sees empty
   values. `release-stable.sh` doesn't notarize either; to notarize a stable build by hand:

   ```sh
   ditto -c -k --keepParent Silas.app Silas.zip
   xcrun notarytool submit Silas.zip --apple-id "$APPLE_ID" --team-id 54H7JX2PD4 \
     --password "$APPLE_PASSWORD" --wait
   xcrun stapler staple Silas.app
   ```

   See Apple's
   [Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

### Moving to Developer ID once

Switching from Apple Development or Silas Local Signing to Developer ID changes the designated
requirement one time. Every guard above refuses that change by design, so plan it:

1. Release with `ALLOW_NEW_REQUIREMENT=1 scripts/release-stable.sh ...`. The release commit
   carries the new line in `scripts/release-requirement.txt`, captured from the real build by
   `codesign`. Never write that line by hand.
2. The in-app updater on existing installs refuses this release ("signed differently from your
   install"). Existing users move with
   `curl -fsSL https://raw.githubusercontent.com/baney75/silas/main/install.sh | SILAS_ALLOW_NEW_SIGNATURE=1 sh`
   or the DMG.
3. Put this in the release notes, once: "This update uses a new Apple signing certificate.
   Allow Silas in Accessibility one more time when asked." If Silas then shows "can't hear your
   key", Fix Permission clears the old row and asks.
4. Signed nightlies fail the requirement check until this release moves the pin. After it,
   grants persist across stable and nightly updates.
