# pm-sysd-w User Guide

pm-sysd-w (PM Sysd (Windows)) is a systemd-style process manager for Windows.
Use `pm-sysd-ctl` to manage processes (services) on Windows, similar to
`systemctl`.

Note: `service` in this documentation is not the Windows service concept. It is
a pm-sysd-w unit type, similar to systemd's service concept; in practice it is
one (or a group of) process(es) started by `pm-sysd-init`.

## Installation

Requires the Rust toolchain (edition 2024).

Only Windows 10 or newer is supported.

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

Note: the current version does not include auto-start at boot (running as a
Windows service) or auto-start after user login; these must be handled manually
by the user. For example, pm-sysd-w will not automatically install
`pm-sysd-init` as a Windows service, nor set up login auto-start; it only starts
and manages services.

`--system` / `--user` / `--machine=NAME` cannot be combined; specify only one.

When `pm-sysd-ctl` / `pm-sysd-log` do not specify an instance option, they try
`--system` first; if the system instance is not running, they automatically fall
back to `--user`. This differs from systemd's default behavior. When connecting
to `--system`, ordinary (non-administrator) users automatically use a read-only
connection and can only make read-only queries (such as viewing logs / status).

## Limitations on Windows Shutdown / Logoff

Because of Windows limitations, on Windows shutdown or user logoff pm-sysd-w
cannot guarantee (in practice, it is almost impossible to implement) a graceful
service stop:

- During shutdown / logoff, Windows does not allow starting new processes /
  running new programs, so `ExecStop` most likely cannot run.
- Recent Windows (Windows 10 / 11) cannot block shutdown; the system forces a
  shutdown. Also, each program has only a very short fixed hard timeout (e.g. 5
  seconds). That is, Windows waits at most 5 seconds for each program, then
  force-terminates. This behavior cannot be configured, and programs cannot
  change it.

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

## Log Notes

- Logs are kept for the last 72 hours by default; pm-sysd-init runs an automatic
  cleanup once per hour.
- A service process's stdout / stderr is written line-by-line to that unit's
  logs. Over-long lines are split into multiple log entries; output is not
  guaranteed to be UTF-8, and on decode failure that line is replaced with one
  error log entry written to the unit.

## Graceful Exit

- `pm-sysd-ctl ... exit` exits pm-sysd-init normally.
- `pm-sysd-ctl ... soft-reboot` restarts pm-sysd-init itself and all managed
  services (not a Windows reboot).
- External signals: SIGTERM on Linux and Ctrl+C on Windows also trigger a
  graceful exit.
