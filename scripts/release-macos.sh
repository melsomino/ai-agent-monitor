#!/bin/bash
# Build and notarize a release locally; does not publish it to GitHub.
set -euo pipefail

if [[ "${1:-}" == "--help" ]]; then
  cat <<'HELP'
Usage: ./scripts/release-macos.sh

Requires macOS, Rust/Cargo, Xcode tools, a Developer ID Application signing
identity in Keychain, and credentials previously saved with:
  xcrun notarytool store-credentials ai-agent-monitor-notary \
    --apple-id YOUR_EMAIL --team-id YOUR_TEAM_ID

Optional environment variables:
  AI_AGENT_MONITOR_SIGN_IDENTITY  Skip prompt; use an available Developer ID identity
  AI_AGENT_MONITOR_NOTARY_PROFILE Keychain profile (default: ai-agent-monitor-notary)

Lists valid Developer ID Application certificates and prompts for a selection.
Builds the current architecture. Produces a signed, notarized and stapled DMG
and SHA256SUMS in build/release. Does not create a tag or upload a release.
HELP
  exit 0
fi
if [[ $# -ne 0 ]]; then
  echo "Unknown argument. Use --help." >&2
  exit 1
fi

[[ "$(uname -s)" == "Darwin" ]] || { echo "This script requires macOS." >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

profile="${AI_AGENT_MONITOR_NOTARY_PROFILE:-ai-agent-monitor-notary}"
for tool in cargo codesign security xcrun hdiutil shasum plutil spctl sed; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 1; }
done
xcrun --find notarytool >/dev/null
xcrun --find stapler >/dev/null

# Only list usable Developer ID Application identities, not development or
# installer certificates. security requires the matching private key to be present.
identity_output=$(security find-identity -v -p codesigning)
identity_hashes=()
identity_names=()
identity_count=0
while IFS='|' read -r hash name; do
  [[ -n "$hash" ]] || continue
  identity_hashes[$identity_count]="$hash"
  identity_names[$identity_count]="$name"
  identity_count=$((identity_count + 1))
done <<< "$(printf '%s\n' "$identity_output" | sed -nE 's/^[[:space:]]*[0-9]+\) ([[:xdigit:]]{40}) "(Developer ID Application:.*)"[[:space:]]*$/\1|\2/p')"

if [[ "$identity_count" -eq 0 ]]; then
  echo "No valid Developer ID Application signing identities found in Keychain." >&2
  echo "Create one in Xcode → Settings → Accounts → Manage Certificates." >&2
  exit 1
fi

echo "Available Developer ID Application certificates:"
for ((i = 0; i < identity_count; i++)); do
  printf '  %d) %s\n     %s\n' "$((i + 1))" "${identity_names[$i]}" "${identity_hashes[$i]}"
done

selected=-1
if [[ -n "${AI_AGENT_MONITOR_SIGN_IDENTITY:-}" ]]; then
  for ((i = 0; i < identity_count; i++)); do
    if [[ "$AI_AGENT_MONITOR_SIGN_IDENTITY" == "${identity_hashes[$i]}" ||
          "$AI_AGENT_MONITOR_SIGN_IDENTITY" == "${identity_names[$i]}" ]]; then
      selected=$i
      break
    fi
  done
  if [[ "$selected" -lt 0 ]]; then
    echo "AI_AGENT_MONITOR_SIGN_IDENTITY must match an identity listed above." >&2
    exit 1
  fi
else
  while [[ "$selected" -lt 0 ]]; do
    if ! read -r -p "Certificate number (q to cancel): " choice; then
      echo "No certificate selected. Set AI_AGENT_MONITOR_SIGN_IDENTITY for unattended use." >&2
      exit 1
    fi
    [[ "$choice" != "q" && "$choice" != "Q" ]] || exit 1
    for ((i = 0; i < identity_count; i++)); do
      if [[ "$choice" == "$((i + 1))" ]]; then
        selected=$i
        break
      fi
    done
    if [[ "$selected" -lt 0 ]]; then
      echo "Enter a number from 1 to $identity_count, or q to cancel." >&2
    fi
  done
fi
export AI_AGENT_MONITOR_SIGN_IDENTITY="${identity_hashes[$selected]}"
printf 'Signing with: %s\n' "${identity_names[$selected]}"

case "$(uname -m)" in
  arm64) architecture=apple-silicon ;;
  x86_64) architecture=intel ;;
  *) echo "Unsupported architecture." >&2; exit 1 ;;
esac

echo "Building and signing the app and creating the DMG…"
cargo run --release --locked -- dmg
app="build/AI Agent Monitor.app"
image="build/release/AI-Agent-Monitor-${architecture}.dmg"
codesign --verify --deep --strict --verbose=2 "$app"

# The app was signed by the bundle command. Sign the outer distribution too.
codesign --force --sign "$AI_AGENT_MONITOR_SIGN_IDENTITY" --timestamp "$image"
codesign --verify --strict --verbose=2 "$image"

# cargo's initial checksum is obsolete after signing/stapling. Only recreate it
# once every release check succeeds, so a rejected image has no release checksum.
rm -f build/release/SHA256SUMS
mkdir -p build/notarization
receipt=$(mktemp "$root/build/notarization/submission.XXXXXX")
echo "Submitting to Apple using Keychain profile: $profile"
if ! xcrun notarytool submit "$image" --keychain-profile "$profile" \
  --wait --output-format json > "$receipt"; then
  cat "$receipt"
  echo "Submission failed. Receipt: $receipt" >&2
  exit 1
fi
cat "$receipt"
status=$(plutil -extract status raw -o - "$receipt")
if [[ "$status" != "Accepted" ]]; then
  submission_id=$(plutil -extract id raw -o - "$receipt")
  echo "Notarization status: $status. Fetching Apple's log…" >&2
  xcrun notarytool log "$submission_id" --keychain-profile "$profile" \
    "$receipt.log.json" || true
  echo "Release stopped. Receipt: $receipt; log: $receipt.log.json" >&2
  exit 1
fi

echo "Stapling and validating the distribution…"
xcrun stapler staple "$image"
xcrun stapler validate "$image"
codesign --verify --strict --verbose=2 "$image"
hdiutil verify "$image"
spctl --assess --type open --context context:primary-signature --verbose=2 "$image"

(
  cd build/release
  shasum -a 256 "$(basename "$image")" > SHA256SUMS
  shasum -a 256 --check SHA256SUMS
)
echo "Ready to upload: $root/$image"
echo "Checksum: $root/build/release/SHA256SUMS"
echo "Do not modify the DMG after this step. Upload it together with SHA256SUMS."
