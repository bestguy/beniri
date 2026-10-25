#!/usr/bin/env bash
# Install BeNiri for the current user and point the niri systemd
# user session at it. No root needed. Takes effect at the next login.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
bin="$HOME/.local/bin/beniri"
cfg="$HOME/.config/niri/config-beniri.kdl"
dropin_dir="$HOME/.config/systemd/user/niri.service.d"

[ -x "$repo/target/release/niri" ] || { echo "build first: cargo build --release -p niri"; exit 1; }
[ -f "$cfg" ] || { echo "missing $cfg"; exit 1; }

install -Dm755 "$repo/target/release/niri" "$bin"
"$bin" validate -c "$cfg"

mkdir -p "$dropin_dir"
cat > "$dropin_dir/beniri.conf" <<CONF
[Service]
ExecStart=
ExecStart=$bin --session -c $cfg
CONF
systemctl --user daemon-reload
echo "installed $bin; niri.service now starts BeNiri at next login"
systemctl --user cat niri.service | grep -A2 "beniri.conf"
