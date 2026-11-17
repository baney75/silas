# Release channels

Silas ships on the three channels every Local Studio product uses: **stable**, **nightly** and
**dev**. The scheme is [docs/channels.md in baney75/local-studio](https://github.com/baney75/local-studio/blob/main/docs/channels.md);
where this page and that one disagree, that one wins. This page records only what is particular
to Silas.

| Channel | Version | Feed | Built by |
| --- | --- | --- | --- |
| stable | `X.Y.Z` | `releases/latest/download/latest.json` | `promote.yml` (checks, Windows, Linux) and `scripts/release-stable.sh` on the release Mac (macOS, every signature, the `stable` move, the tag) |
| nightly | `X.Y.Z-nightly.YYYYMMDDHHMMSS+sha7` | `releases/download/nightly/latest.json` | `nightly.yml`, 07:00 UTC when `main` changed |
| dev | `X.Y.Z-dev.YYYYMMDDHHMMSS+sha7` | `releases/download/dev/latest.json` | `dev.yml`, every merge to `main` |

All three workflows call `channel-build.yml`, which builds into a private draft and publishes
nothing until every platform built. `scripts/channel-version.sh` makes the nightly and dev
versions and refuses to publish a build that is not newer than the feed.
`scripts/check-channels.sh` runs in CI on every pull request; `scripts/check-channels-test.sh`
proves it fails on bad workflows.

## Install mode: replace

One app id (`com.baney.silas`) for every channel, so switching channel replaces the install.
macOS ties the Accessibility and Microphone grants to the app's signature (docs/signing.md), and
a second id would make the user grant them again. Silas's updater, unlike the shared scheme's,
does cross channels: the Settings picker sets which feed it reads, and it installs whatever that
feed offers when the version differs, even a lower one. Each feed is still checked against its
own key (`src-tauri/src/updates.rs`): stable against the stable key, nightly and dev against the
nightly key.

## Stable is signed on the Mac

The stable updater key never leaves the release Mac, and the Mac's signing keychain needs a GUI
session there (field note `silas-stable-release-over-ssh-fails-codesign`). So stable is split:

1. The lead runs `gh workflow run promote.yml --ref main -f nightly=nightly-YYYYMMDDHHMMSS`
   (or `-f nightly=nightly` for the newest). It checks the promotion conditions (72 hours old,
   carries `stable`'s head, no open `regression` issue) and builds the Windows and Linux bundles
   into a draft release for `vX.Y.Z` whose target is the nightly's commit. Their updater bundles
   are unsigned: no CI job ever holds a stable key, and `check-channels.sh` fails any workflow
   that names one.
2. On the release Mac, in a Terminal: `git fetch origin && git checkout --detach <commit> &&
   scripts/release-stable.sh X.Y.Z "notes"`, with the commit from the run's summary. It builds
   and signs the Mac app as before, signs the Windows and Linux bundles from the draft with the
   stable key, writes one `latest.json` for every platform and uploads it all. Only then does it
   fast-forward `stable` to the commit, tag `vX.Y.Z` there, check the tag equals `stable`'s head,
   and publish the draft as the latest release.
3. Bump `main`'s base version (below).

This is where Silas differs from the shared scheme, which has `promote.yml` move `stable` and
push the tag. A workflow's own token can't push a ref update that carries changes to
`.github/workflows/`, and it can't be given the permission to. A promotion nearly always carries
some, so the move and the tag happen in `release-stable.sh`, with the release operator's own
credentials, which also sign the release.

**Hotfix**: a pull request into `stable`, reviewed and merged by a repository admin with a merge
commit, then `gh workflow run promote.yml --ref main -f hotfix=true`, which builds `stable`'s
head as the next patch, then step 2 (`stable` is already at the commit, so only the tag moves).
The hotfix path skips the open-regression check, since fixing one is the usual reason for a
hotfix and a pull request into `stable` never closes its issue. Merge `stable` back into `main`
with a merge commit the same day. If the last release changed the signing requirement
(`ALLOW_NEW_REQUIREMENT=1`), the hotfix pull request also carries that release's
`scripts/release-requirement.txt`, or `release-stable.sh` checks the hotfix against the old pin.

The `stable` branch and its rulesets are set up by hand once. The branch starts at the commit
that merged this scheme into `main` (it contains v0.1.6, and every later nightly descends from
it). Rulesets: `stable` can't be deleted or force-pushed, and only repository admins may update
it (`release-stable.sh` or a hotfix merge); only repository admins may create `v*` tags.

## Versions

The base `X.Y.Z` is `version` in `src-tauri/tauri.conf.json`. Nightly and dev bump its patch, so
with base `0.1.6` they are `0.1.7-nightly…` and `0.1.7-dev…`, and promoting one gives stable
`0.1.7`. The nightlies published before this scheme used minutes and no `+sha7`
(`0.1.7-nightly.202611081355`); the new ones sort above them (`scripts/channel-version-test.sh`).

**After a stable release**, bump the base in a pull request to `main` to the next minor:
`0.1.7` ships, `main` sets `0.2.0`, so nightlies become `0.2.1-nightly…` and the next stable
`0.2.1`. Hotfixes on `0.1.7` take `0.1.8`, a number `main` never reaches. `promote.yml` does not
do this itself: a pull request opened by the workflow's token would not run CI.

`+sha7` is SemVer build metadata. Every bundler accepts it (checked against tauri-bundler 2.12.2):
NSIS writes `X.Y.Z.0` for the Windows file version and warns that the build metadata is not a
number; deb, rpm, AppImage and the macOS `Info.plist` carry the string as is; the updater parses
it with the `semver` crate. GitHub release asset names can't hold `+`, so uploads write it as `.`.

## Windows and Linux on stable

Until a stable release carries Windows and Linux, the stable feed has no entry for them, and a
Windows or Linux install set to stable checks nightly instead (`falls_back_to_nightly` in
`updates.rs`). Once a stable release carries the platform, the same setting gets stable with no
change to the app. The installers (`install.sh` on Linux, `install.ps1`) install nightly.

## Other releases on this repo

The stable feed is `releases/latest`, so only a stable app release may be marked latest. Any
other release on this repo (benchmark sets, models, test builds) is created with
`--latest=false` (`gh release create … --latest=false`). `check-channels.sh` fails a workflow or
script that publishes a release without saying `--latest`, `--latest=false` or `make_latest`.
