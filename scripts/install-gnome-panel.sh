#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
uuid=ai-agent-monitor@melsomino
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
cp "$root/src/linux/gnome-extension/"*.js "$root/src/linux/gnome-extension/metadata.json" \
  "$root/src/linux/gnome-extension/"*.svg "$stage/"
(cd "$stage" && zip -q "$stage/panel.zip" ./*.js metadata.json ./*.svg)
gnome-extensions install --force "$stage/panel.zip"
if ! gnome-extensions enable "$uuid"; then
  enabled=$(gsettings get org.gnome.shell enabled-extensions)
  if [[ "$enabled" != *"'$uuid'"* ]]; then
    if [[ "$enabled" == '[]' || "$enabled" == '@as []' ]]; then
      enabled="['$uuid']"
    else
      enabled="${enabled%]}, '$uuid']"
    fi
    gsettings set org.gnome.shell enabled-extensions "$enabled"
  fi
  printf '%s\n' 'Installed and queued for enablement. GNOME requires one logout/login to load a new extension.'
  exit 0
fi
gnome-extensions info "$uuid"
expected=$(sed -n 's/.*"version": *\([0-9][0-9]*\).*/\1/p' "$root/src/linux/gnome-extension/metadata.json")
loaded=$(gnome-extensions info "$uuid" | sed -n 's/^ *Version: *//p')
if [[ "$loaded" != "$expected" ]]; then
  printf '%s\n' 'The updated panel code will load after one logout/login.'
fi
