# Contributing to yalt

Thanks for helping improve yalt. Bug reports, reproducible format issues, and
focused pull requests are welcome.

## Development setup

yalt currently targets Apple Silicon Macs running macOS 12 or later. Install
Node.js 22+, Rust 1.77.2+, and the Xcode command-line tools, then run:

```sh
npm install
npm run tauri dev
```

## Before opening a pull request

Run the complete local verification suite:

```sh
npm run check
```

Please keep changes focused, add regression coverage for behavior or format
changes, and update the README when user-facing workflows change. Test import
and export changes with small fixtures that contain no private dataset content.

## Privacy

yalt is intentionally local-only. Contributions must not add telemetry,
analytics, remote image processing, or network uploads without an explicit and
well-documented project decision.

## Reporting problems

Include the yalt version, macOS version, task type, relevant import/export
format, and concise reproduction steps. Do not attach private training images
or project databases to public issues.
