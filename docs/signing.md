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
3. the first `Apple Development:` identity.

Prefer Developer ID Application: its requirement pins the Team ID, which survives certificate
renewal. Switching from Apple Development to Developer ID changes the requirement once, so
users are asked once more after that release.

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

`.github/workflows/nightly.yml` signs the Mac build with a real certificate when the repo has
these secrets, and falls back to ad-hoc with a warning annotation when it doesn't:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of a `.p12` export of a **Developer ID Application** certificate and its private key (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | the password set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | the identity's full name, e.g. `Developer ID Application: Donovan Baney (TEAMID)` |

CI cannot create a stable identity on its own. The Team ID comes from Donovan's Apple Developer
Program membership, and only a certificate issued to that team gives a requirement that holds
from build to build. Until the three secrets exist, Mac nightlies stay ad-hoc and each nightly
update asks for Accessibility again.

Use the same Developer ID certificate as the stable channel, so moving between channels keeps
the grants too.

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
