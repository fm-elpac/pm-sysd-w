# pm-sysd-w User Guide

pm-sysd-w (PM Sysd (Windows)) is a systemd-style process manager for Windows.
Use `pm-sysd-ctl` to manage processes (services) on Windows, similar to
`systemctl`.

## Installation

Requires the Rust toolchain (edition 2024).

```sh
cargo build --release
# binaries at target/release/pm-sysd-init, target/release/pm-sysd-ctl and
# target/release/pm-sysd-log
```

## Quick Start

First start an instance (custom instance `dev` here) with a centralized config:

```sh
pm-sysd-init --machine=dev --conf=pm_sysd_w.conf.toml
```

Then operate with `pm-sysd-ctl` (specifying the same instance):

```sh
pm-sysd-ctl --machine=dev daemon-reload

pm-sysd-ctl --machine=dev start test-run.service

pm-sysd-ctl --machine=dev status test-run

pm-sysd-ctl --machine=dev enable test-run.service
```

## Instances

Multiple pm-sysd-init instances may run on one system simultaneously. Select the
instance to operate on with `pm-sysd-ctl`'s instance option:

| Option           | Instance        | Description                                                            |
| ---------------- | --------------- | ---------------------------------------------------------------------- |
| `--system`       | System instance | Usually runs as a Windows service, auto-starts at boot; requires admin |
| `--user`         | User instance   | Auto-starts after the current user logs in                             |
| `--machine=NAME` | Custom instance | Started manually, with a custom name                                   |

## Config File

pm-sysd-w uses pm-tc's centralized TOML config file, with many units written in
one file. The format version is fixed at `pm_tc = 1`. See [config.md](config.md)
for the full field reference.

Example (see `example/pm_sysd_w.conf.toml`):

```toml
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test-run.service"
"env:Unit.Description" = "Run a test program"

"env:Service.ExecStart" = "path/to/bin --arg"

"env:Service.Restart" = "always"
"env:Service.RestartSec" = "10"

"env:Install.WantedBy" = "default.target"
```

- Each `[[t.sysd]]` defines one unit (service).
- `env:Unit`: unit name.
- `env:Unit.Description`: description.
- `env:Service.ExecStart`: start command.
- `env:Service.Restart` / `env:Service.RestartSec`: restart policy.
- `env:Install.WantedBy`: the target the unit belongs to (initially only
  built-in targets such as `default.target`).
- `env:Win.CreateNoWindow`: (optional, Windows only) set CREATE_NO_WINDOW when
  starting, so console programs show no black window; ignored on Linux. Default
  `no`.

## pm-sysd-ctl Commands

Supported commands (initial version), similar in style to `systemctl` (some may
just print `TODO` in the initial version):

```sh
list-units
list-timers
is-active
is-failed
status
show
cat
list-dependencies

start
stop
reload
restart
try-restart
reload-or-restart
try-reload-or-restart
kill
clean
reset-failed
whoami

enable
disable
reenable
preset
is-enabled
mask
unmask
get-default
set-default

show-environment
set-environment
unset-environment
import-environment

daemon-reload
soft-reboot
exit
```

`enable` / `disable` support `--now` (e.g. `enable --now`).

Common options:

```sh
-h --help
--system
--user
-M --machine=NAME
--wait
--no-block
```

## pm-sysd-log

`pm-sysd-log` views logs (similar to `journalctl`):

```sh
# show recent logs (jump to the end, default)
pm-sysd-log --machine=dev

# show recent logs for the specified unit only (-eu combined)
pm-sysd-log --machine=dev -eu test-run.service
```

Common options:

```sh
-h --help
-e  # jump to the end (default)
-u --unit=UNIT
-x --catalog  # no effect in the initial version
--system
--user
-M --machine=NAME
```
