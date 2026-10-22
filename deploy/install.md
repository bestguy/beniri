# Installing the fork

The fork builds to the same `niri` binary as upstream, with the same session
files, IPC and config. There are two ways to run it.

## Route 1: install it like niri

Build with `cargo build --release -p niri` and follow upstream's
[Getting Started](../docs/wiki/Getting-Started.md#manual-installation) table:
copy the binary and the files under `resources/` to the system locations
listed there. This replaces your distribution's niri entirely.

## Route 2: run it beside your distribution's niri

This route leaves the distribution package installed and untouched, points
only your own user session at the fork, and can be undone with one script.
No root is needed. It assumes:

* your distribution's `niri` package is installed, since its session files
  and portal config are reused;
* niri is started by the systemd user service `niri.service`, which is how
  Fedora, Arch and most other distributions ship it.

### Set up

1. Build: `cargo build --release -p niri`.
2. Create `~/.config/niri/config-beos.kdl`. It layers the fork's settings on
   top of your regular config, which stays readable by stock niri:

       include "config.kdl"

       layout {
           title-tab {
               // Everything is optional; the tab is on by default.
               // See docs/wiki/Configuration:-Title-Tab.md for all options.
               font "sans bold 10"
           }
       }

3. Run `deploy/install.sh`. It copies the binary to `~/.local/bin/niri-beos`,
   validates the config, and writes a systemd drop-in at
   `~/.config/systemd/user/niri.service.d/override.conf` that makes
   `niri.service` run

       ~/.local/bin/niri-beos --session -c ~/.config/niri/config-beos.kdl

4. Log out and in. The session now runs the fork.

The packaged `niri msg` CLI keeps working against the fork because the IPC is
unchanged. Shells and theming tools that write into your niri config keep
working too, since `config.kdl` is still the file they edit.

### Going back

Run `deploy/rollback.sh`, then log out and in. It only removes the drop-in;
the fork binary and `config-beos.kdl` are left in place, unused.

If a login ever fails, do the same from a TTY: log in, delete the
`override.conf` file, run `systemctl --user daemon-reload`, and log in again.

### Updating

After pulling or rebuilding, run `deploy/install.sh` again and re-login.
The script replaces the binary with `install(1)`, which is safe while the old
one is running.

If your distribution updates its niri package, your session keeps running the
fork, but the packaged `niri msg` may then be newer than the fork's IPC.
Rebase the fork onto the new upstream release, rebuild and reinstall.

## Turning the tab off

Add this to your config for stock niri rendering, no reinstall needed:

    layout {
        title-tab {
            off
        }
    }
