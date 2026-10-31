# Project instructions

## macOS release notarization

- `scripts/release-macos.sh` prepares a signed, notarized, and stapled release locally. It does not publish to GitHub.
- Run this script only when the user explicitly instructs you to run it or notarize a release, before uploading release artifacts to GitHub.
- Do not run notarization automatically during builds, tests, app signing, DMG packaging, verification, commits, pushes, or release publication. A request for those operations alone does not authorize this script.
- Keep ordinary `bundle` and `dmg` signing separate from this explicitly requested notarization step.
