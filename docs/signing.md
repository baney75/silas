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

`scripts/release-stable.sh` signs on Donovan's Mac. It picks, in order:

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
rewrites `scripts/release-requirement.txt` to what `codesign` prints, commits it with the
release, and every later Local Signing build must match it. Users grant Accessibility once more
after that release, then never again for Mac-built releases.

The script prints the new build's requirement, refuses an ad-hoc one, and stops if it differs
from the installed `/Applications/Silas.app` (or `~/Applications/Silas.app`). Set
`ALLOW_NEW_REQUIREMENT=1` for an intended change such as that one-time switch.

`scripts/release-requirement.txt` pins the requirement of the last stable release (v0.1.6,
Apple Development). `scripts/check-requirement.sh` compares every build to it: the release
script stops on a mismatch, a signed nightly fails before upload, and CI checks the pin itself.
Only a stable release with `ALLOW_NEW_REQUIREMENT=1` rewrites the pin, and the release commit
carries the new line. An ad-hoc installed copy holds no grants worth keeping, so the release
script skips it and goes by the pin. Once the Developer ID secrets exist, signed nightlies fail
until the first Developer ID stable release moves the pin.

## Nightly channel

`.github/workflows/nightly.yml` publishes a Mac nightly only when it is signed with a real
certificate from these three secrets. Without them it still builds the Mac app, to prove it
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

## A switch that looks on but isn't

When a build with a different requirement replaces Silas, System Settings can still show Silas
on under Accessibility, because the entry belongs to the old signature. `AXIsProcessTrusted` says
no for the new process, and Silas shows "Silas can't hear your key yet". The banner tells the
user to turn the switch off and on, or remove Silas and add it again. Open Settings goes
straight to Privacy & Security, Accessibility, and never resets anything. The home screen
re-reads the permission every second while the banner shows, and the key listener retries every
two seconds, so the banner clears without a restart. Only "Reset it and ask again" in onboarding
clears Silas's TCC entry.

Accessibility is the gate for the key listener. Silas does not read Input Monitoring.

## Asking for permissions

Separately from signing, Silas asks as little as it can (`src-tauri/src/permissions.rs`):

- Launch, `get_state` and the onboarding poll only read (`AXIsProcessTrusted`,
  `authorizationStatusForMediaType`). They never show a prompt.
- A click on Allow reads first. If access is already granted, no system prompt is shown.
- Each system prompt is shown at most once per run. A second click while still denied opens
  the Settings pane instead.
- "Reset it and ask again" is the one path that clears Silas's TCC entry, and only when the
  read says Silas is not trusted.

Every read and every request or skip logs a `silas.permissions` line, so a run shows
"preflight accessibility=Granted; no request" without touching TCC.

## Follow-up

Notarization (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` for `notarytool`) is not set up.
It isn't needed for TCC grants to persist, but Gatekeeper warns on first open of a
Developer ID build that isn't notarized.
