# SilasCore

Pure Swift shared by the Apple apps: filler and stutter cleanup (a port of the desktop's
`drop_fillers` and `drop_stutters`), the hold and tap state machine, short-clip padding, history,
and the Obsidian URL. No dependencies.

Pin it from another app by revision or tag; the manifest SwiftPM reads is `Package.swift` at the
repo root, which points here:

```swift
.package(url: "https://github.com/baney75/silas", revision: "<sha>")
```

Check: `swift test` at the repo root, or in this folder. macOS CI runs it.

Source: the `ipad-app` branch (947b6a8), reviewed there in three rounds.
