# pm-sysd-w User Guide

pm-sysd-w (PM Sysd (Windows)) manages processes on Windows in a systemd-like
way.

Use `pm-sysd-ctl` to operate services, similar to `systemctl`; use `pm-sysd-log`
to view logs, similar to `journalctl`.

pm-sysd-w is not systemd-compatible and implements only a small subset of
systemd. This guide focuses on the **differences** from systemd; parts that
match (or closely resemble) systemd are simplified or omitted.

## Concepts

`service` is a pm-sysd-w unit type, similar in concept to systemd's service.
Note that it is not the Windows "Windows service"; in practice it is one (or a
group of) process(es) started and managed by `pm-sysd-init`.

`pm-sysd-init` is a resident daemon (similar to systemd's PID 1 role) that
starts and manages services. `pm-sysd-ctl` / `pm-sysd-log` communicate with it
(the instance) to perform operations.

## Key Differences from systemd

- Config file: uses pm-tc's **centralized** TOML format, with multiple units in
  one file, instead of systemd's multiple unit files.

- Deployment: by default the current version does not include auto-start at boot
  / after login, and does not install `pm-sysd-init` as a Windows service or set
  up login auto-start. The experimental `install/setup.ps1` is available (see
  `Installation`); other auto-start methods must still be handled manually.

- Default instance: when `pm-sysd-ctl` / `pm-sysd-log` do not specify an
  instance, they try `--system` first; if the system instance is not running,
  they fall back to `--user`. This differs from systemd (systemd connects only
  to the system instance by default and does not fall back).

- Permissions: an ordinary (non-administrator) user connecting to `--system`
  automatically uses a read-only connection and can only make read-only queries
  (such as viewing status / logs).

- The `cat` command prints JSON, not systemd's unit file content.

- `soft-reboot` restarts `pm-sysd-init` itself and all managed services, not
  Windows.

- On Windows shutdown / logoff, a graceful service stop cannot be guaranteed
  (see below).

- Logs are stored in a local database and kept for the last 72 hours by default;
  there is also a pm-sysd-w-specific command `sqlite-vacuum` for cleaning /
  optimizing the database.

## Installation

You must currently build from source. A Rust toolchain (edition 2024) is
required; only Windows 10 or newer is supported.

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

The binaries are under `target/release/`: `pm-sysd-init.exe`, `pm-sysd-ctl.exe`,
`pm-sysd-log.exe`.

### Install Script (Experimental)

The project provides `install/setup.ps1`, run with PowerShell from the project
root. It currently supports two commands, `install-user` and `uninstall-user`,
and requires administrator privileges.

```powershell
# run from the project root
powershell -ExecutionPolicy Bypass -File install\setup.ps1 install-user
powershell -ExecutionPolicy Bypass -File install\setup.ps1 uninstall-user
```

`install-user`:

1. Copies `pm-sysd-init.exe` / `pm-sysd-ctl.exe` / `pm-sysd-log.exe` to
   `%LOCALAPPDATA%\pm-sysd-w\bin\`.
2. Adds that directory to the current user's `PATH`.
3. Writes an empty config file to `%LOCALAPPDATA%\pm-sysd-w\pm_sysd_w.conf.toml`
   (does not overwrite an existing file).
4. Installs `pm-sysd-init.exe --user` as a Windows per-user service (started
   automatically when the user signs in).

`uninstall-user` only deletes that per-user service and leaves the
`%LOCALAPPDATA%\pm-sysd-w\` directory untouched.

> **Experimental support**: only these two commands are supported for now.
> pm-sysd-init now implements the Windows service interface, so the per-user
> service starts automatically when the user signs in. The service requires an
> explicit `--conf` path (the install script passes it). Other auto-start
> methods (Task Scheduler / Startup folder / Run registry key) are still handled
> manually by the user.

## Quick Start

First start an instance (here, the current-user instance `--user`), loading the
centralized config file:

```sh
pm-sysd-init --user --conf=pm_sysd_w.conf.toml
```

Then operate with `pm-sysd-ctl` (specifying the same instance), for example:

```sh
pm-sysd-ctl --user start test-run

pm-sysd-ctl --user status test-run

pm-sysd-ctl --user enable test-run
```

View logs and exit the instance:

```sh
pm-sysd-log --user -eu test-run.service

pm-sysd-ctl --user exit
```

## Instances

Multiple `pm-sysd-init` instances may run on one system simultaneously. Select
the instance to operate on with `pm-sysd-ctl`'s instance option:

| Option           | Instance        | Description                                                    |
| ---------------- | --------------- | -------------------------------------------------------------- |
| `--system`       | System instance | Machine-wide, requires admin                                   |
| `--user`         | User instance   | Current logged-in user                                         |
| `--machine=NAME` | Custom instance | Started manually; NAME uses only [0-9A-Za-z_-], length 1..=100 |

- When starting, `pm-sysd-init` must specify `--conf=PATH` (the main config
  file) and an instance option; `--system` / `--user` / `--machine=NAME` cannot
  be combined, and only one may be specified.

- By default the current version does not include auto-start at boot / after
  login; the above startup methods must be handled manually (for example Task
  Scheduler / Startup folder / Run registry key). See `Installation` for the
  experimental `install/setup.ps1`.

- When `pm-sysd-ctl` / `pm-sysd-log` do not specify an instance option, they try
  `--system` first; if the system instance is not running, they automatically
  fall back to `--user`. When connecting to `--system`, ordinary
  (non-administrator) users automatically use a read-only connection.

## Config File

pm-sysd-w uses pm-tc's centralized TOML config file, with multiple units in one
file. The format version is fixed at `pm_tc = 1`. See [config.md](config.md) for
the full field reference.

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
- `env:Install.WantedBy`: the target the unit belongs to; in the initial version
  only a unit with value `default.target` can be `enable`d.
- `env:Win.CreateNoWindow`: (optional, Windows only) set `CREATE_NO_WINDOW` when
  starting, so console programs show no black window; ignored on Linux. Default
  `no`.

## pm-sysd-ctl Commands

Supported commands (initial version), similar in style to `systemctl` (some
commands may just print `TODO` in the initial version):

```sh
list-units
list-timers

status
is-active
is-failed
is-enabled
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
reset-failed
clean

enable
disable
reenable
mask
unmask

daemon-reload
get-default
set-default
preset

show-environment
set-environment
unset-environment
import-environment

whoami

soft-reboot  # restart pm-sysd-init itself and all managed services
exit  # exit pm-sysd-init

sqlite-vacuum  # clean / optimize the sqlite database
```

Common options:

```sh
   --version
-h --help

   --system
   --user
-M --machine=NAME

   --wait
   --no-block
```

Command behavior notes:

- `enable` / `disable` support `--now` (for example `enable --now`).

- In the initial version, `preset` is equivalent to `disable`: it sets the unit
  to disabled; units that do not support enable/disable (no `[Install]` section)
  are ignored and succeed.

- Unit names without a suffix get `.service` appended automatically (for example
  `status test` equals `status test.service`); the same applies to
  `pm-sysd-log -u` / `--unit`.

- `mask` behavior matches systemd: it supports `--now` (by default it only
  masks, and stops only with `--now`); a masked unit is treated as nonexistent,
  but a still-running service is shown as `masked`, and commands such as `start`
  refuse to start it.

- `show` and `cat` print the same set of all supported config fields: `show`
  uses `key=value` (keys match the config, e.g. `Service.ExecStart`), while
  `cat` uses pretty JSON (it does not mimic systemd's unit file format).

- The output format of `list-units` / `status` / `list-timers` mimics
  `systemctl` as much as possible; fields not relevant to this project are
  omitted.

- `list-units` sorts by unit name. Units removed from the new config by
  `daemon-reload` but still running are also listed (their state comes from the
  running old config).

- The exit codes of `is-active`, `is-failed` and `is-enabled` match systemd.

- `set-default` initially supports only `default.target`; other values error.

- `daemon-reload` re-reads the config file and refreshes the enable / mask
  state; while a reload is in progress, `enable` / `disable` / `reenable` /
  `mask` / `unmask` report an error, and concurrent `daemon-reload` requests are
  not allowed.

- `list-dependencies` returns an empty result because unit dependencies are not
  implemented yet.

## pm-sysd-log

`pm-sysd-log` views logs (similar to `journalctl`):

```sh
# show recent logs (jump to the end, default)
pm-sysd-log --user

# show recent logs for the specified unit only (-eu combined)
pm-sysd-log --user -eu test-run.service
```

Common options:

```sh
-e  # jump to the end (default)
-u --unit=UNIT
-x --catalog  # no effect in the initial version

   --system
   --user
-M --machine=NAME

-h --help
   --version
```

### Log Notes

- Logs are kept for the last 72 hours only; pm-sysd-init runs an automatic
  cleanup once per hour.

- A service process's stdout / stderr is written line-by-line to that unit's
  logs. Over-long lines are split into multiple log entries; the output is not
  guaranteed to be UTF-8, and a decode failure replaces it with one error log
  entry.

## Exiting and Restarting the Instance

- `pm-sysd-ctl ... exit` exits pm-sysd-init normally.

- `pm-sysd-ctl ... soft-reboot` restarts pm-sysd-init itself and all managed
  services (not a Windows reboot).

- External signals: `SIGTERM` on Linux and `Ctrl+C` on Windows also trigger a
  graceful exit.

## Limitations on Windows Shutdown / Logoff

Because of Windows limitations, on Windows shutdown or user logoff pm-sysd-w
cannot guarantee (in practice, it is almost impossible to implement) a graceful
service stop:

- During shutdown / logoff, Windows does not allow starting new processes /
  running new programs, so `ExecStop` most likely cannot run.

- Recent Windows (Windows 10 / 11) cannot block shutdown; the system forces a
  shutdown. Also, each program has only a very short fixed hard timeout (for
  example 5 seconds). That is, Windows waits at most 5 seconds for each program,
  then forces the shutdown. This behavior cannot be configured, and programs
  cannot change it.
