#!/usr/bin/env bash
# Return the niri systemd user session to the stock /usr/bin/niri. Takes
# effect at the next login. The fork binary and config-beos.kdl are left in
# place and unused.
set -euo pipefail
rm -f "$HOME/.config/systemd/user/niri.service.d/override.conf"
rmdir "$HOME/.config/systemd/user/niri.service.d" 2>/dev/null || true
systemctl --user daemon-reload
echo "niri.service restored to stock niri; log out and in to apply"
