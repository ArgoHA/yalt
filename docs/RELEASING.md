# Releasing yalt

Nothing in this document should be run until the release is intentionally being
published.

## GitHub release

1. Confirm `package.json`, `src-tauri/Cargo.toml`, and
   `src-tauri/tauri.conf.json` contain the same version.
2. Update `CHANGELOG.md` and run `npm run check`.
3. Configure these GitHub Actions secrets for Apple signing and notarization:
   `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
   `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`.
4. Push a `v0.1.0` tag or manually run the Release workflow. The workflow
   creates a draft release; it does not make the release public.
5. Download the draft DMG on a clean Apple Silicon Mac. Verify installation,
   Gatekeeper, project creation, legacy `.labeler` project opening, annotation
   autosave, and each export format.
6. Publish the draft only after the signed and notarized build passes that
   acceptance check.

## Homebrew — deferred

Do not submit yalt to Homebrew yet. After the first GitHub release is public and
stable, create a cask pointing to its versioned DMG, use the DMG's exact SHA-256,
test it with `brew install --cask`, and only then open a Homebrew Cask pull
request or publish a personal tap.
