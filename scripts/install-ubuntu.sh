#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
executable=${1:-"$root/target/release/AIAgentMonitor"}
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_dir="$HOME/.local/bin"
install -Dm755 "$executable" "$bin_dir/AIAgentMonitor"
install -Dm644 "$root/assets/AppIcon-robots.png" "$data_dir/icons/hicolor/256x256/apps/ai-agent-monitor.png"
install -Dm644 "$root/assets/linux/ai-agent-monitor.desktop" "$data_dir/applications/ai-agent-monitor.desktop"
# Desktop launchers may not inherit ~/.local/bin from the graphical shell.
escaped=${bin_dir//\\/\\\\}
escaped=${escaped//\"/\\\"}
escaped=${escaped//%/%%}
printf '[Desktop Entry]\nType=Application\nName=AI Agent Monitor\nComment=Monitor coding agent sessions and answer requests\nExec="%s/AIAgentMonitor" sessions\nIcon=ai-agent-monitor\nTerminal=false\nCategories=Development;Utility;\nStartupNotify=false\n' "$escaped" > "$data_dir/applications/ai-agent-monitor.desktop"
"$root/scripts/install-gnome-panel.sh"
printf '%s\n' "Installed $bin_dir/AIAgentMonitor"
