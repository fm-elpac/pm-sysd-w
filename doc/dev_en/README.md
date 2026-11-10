# pm-sysd-w Developer Documentation

pm-sysd-w (PM Sysd (Windows)) is a systemd-style process manager for Windows. It
mimics the style and user experience of systemd, managing processes (services)
on Windows; it is not systemd-compatible and implements only a small subset of
systemd. The name "稳哒" (Wěndā) comes from the internet phrase "稳稳哒接住你"
("catch you steadily").

## Build Targets

pm-sysd-w supports 2 build targets:

| Target  | Triple                     | Purpose                     |
| ------- | -------------------------- | --------------------------- |
| Windows | `x86_64-pc-windows-msvc`   | Main supported platform     |
| Linux   | `x86_64-unknown-linux-gnu` | For convenient testing only |

The Linux target exists only for convenient testing: Linux already has systemd,
so pm-sysd-w has almost no practical use there. What can be tested on Linux is
only part of pm-sysd-w's full functionality (and may be incomplete); only
testing on Windows covers the full functionality. Because testing on Windows is
more cumbersome, development first runs quick tests on Linux, then performs the
complete validation on Windows.

## Binaries

pm-sysd-w builds 3 binaries:

| Binary         | Role            | Description                                                              |
| -------------- | --------------- | ------------------------------------------------------------------------ |
| `pm-sysd-init` | PID-1-like      | Resident daemon managing processes it starts (services); async via tokio |
| `pm-sysd-ctl`  | systemctl-like  | Main CLI tool; operates by talking to `pm-sysd-init`; synchronous        |
| `pm-sysd-log`  | journalctl-like | Log viewer; reads logs by talking to `pm-sysd-init`; synchronous         |

The 3 binaries map to 3 cargo features:

| Binary         | Feature    |
| -------------- | ---------- |
| `pm-sysd-init` | `cli_init` |
| `pm-sysd-ctl`  | `cli_ctl`  |
| `pm-sysd-log`  | `cli_log`  |

The default features do not include these 3 features. Normally, build the 3
binaries separately, each with its own feature enabled (do not enable two cli
features at the same time):

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

This way `pm-sysd-init --version`, `pm-sysd-ctl --version` and
`pm-sysd-log --version` produce different output (pm-bin's `--version` prints
all enabled features).

## Module Layout

| Module      | Description                                       |
| ----------- | ------------------------------------------------- |
| `cli`       | Common module for CLI programs (cli feature only) |
| `cli::init` | Implements the `pm-sysd-init` binary (CLI)        |
| `cli::ctl`  | Implements the `pm-sysd-ctl` binary (CLI)         |
| `cli::log`  | Implements the `pm-sysd-log` binary (CLI)         |
| `jsonl`     | JSONL communication protocol between init and ctl |
| `conf`      | Calls pm-tc to read the TOML config file          |
| `db`        | SQLite database abstraction/wrapper               |
| `sysd`      | Core service-management logic                     |
| `logd`      | Core logging logic (journald-like, simplified)    |
| `pa`        | Low-level platform (OS) abstraction interface     |
| `p_w`       | `pa` implementation for Windows (main platform)   |
| `p_u`       | `pa` implementation for Linux (testing only)      |

## Config File (depends on pm-tc)

Instead of systemd's "many config files" approach, pm-sysd-w uses a
**centralized** config: many units are written in one TOML file, read by pm-tc
(PM Template Config Tool). pm-tc is published on crates.io as `0.1.0-a3`; source
is in `~/pm-tc/`.

After loading the config, pm-sysd-w calls pm-tc's `task::plan("sysd")` and gets
all unit configs from the returned `PlanDebug`. Each unit corresponds to one
`[[t.sysd]]` task instance in the config file, with unit settings passed via
`env:XXX` fields (example):

```toml
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test-run.service"
"env:Unit.Description" = "Run a test program"

"env:Service.ExecStart" = "path/to/bin --arg"

"env:Service.Restart" = "always"
"env:Service.RestartSec" = "10"

"env:Install.WantedBy" = "default.target"

"env:Win.CreateNoWindow" = "yes"
```

`env:Win.CreateNoWindow` is a pm-sysd-w-specific extension (Windows only,
ignored on Linux), corresponding to `[Win] CreateNoWindow=` in a systemd unit
file. Boolean fields support the systemd syntax: `1` / `yes` / `true` / `on`
mean true, and `0` / `no` / `false` / `off` mean false.

## Communication (IPC)

How `pm-sysd-init` and `pm-sysd-ctl` / `pm-sysd-log` communicate:

- Linux: a UNIX socket in the current directory (testing only; default
  permissions, no special handling needed).
- Windows: a named pipe, requiring strict access control; only local programs
  may connect, network connections are forbidden.

## Wire Protocol (JSONL)

Data transfer is JSONL-based (one JSON object per line), using `serde_json` for
encoding/decoding.

After ctl / log connects to init, it first performs a version handshake (client
sends, init replies). Handshake message format:

```json
{ "pm-sysd-w": 1, "data": { "version": "0.1.0-a1" } }
```

- `pm-sysd-w`: the wire protocol format version (envelope schema version), fixed
  at `1`.
- `data.version`: the pm-sysd-w software version (string, semver), used for
  version checking.

Handshake flow:

1. After connecting, the client (ctl / log) sends the handshake message above
   (carrying its own `data.version`).
2. init replies with a handshake message of the same shape (carrying its own
   `data.version`).
3. Each side compares the peer's `data.version` with its own; on mismatch it
   must report an error and exit rather than continue.

Only the handshake message carries a version number; later messages do not need
one.

The wire protocol is an internal API and is not provided as a stable external
API; even minor version updates may introduce BREAKING changes. In principle,
the init and ctl / log versions should match.

## pm-sysd-init

Multiple pm-sysd-init instances may run on one Windows system simultaneously.
Named pipes are used for instance uniqueness detection (a same-named
pm-sysd-init can only have 1 process; starting a duplicate reports an error and
exits).

Named pipe name prefix: `pm_sysd_w_*`.

| Instance           | Start arg       | Named pipe                                   | Permission                            |
| ------------------ | --------------- | -------------------------------------------- | ------------------------------------- |
| System             | `--system`      | `pm_sysd_w_system_default`                   | Administrators only                   |
| System (read-only) | `--system`      | `pm_sysd_w_system_default_ro`                | Local normal users: read-only queries |
| User               | `--user`        | `pm_sysd_w_user_NAME` (NAME is the username) | Current user only                     |
| Custom             | `--machine=XXX` | `pm_sysd_w_m_XXX`                            | Current user only                     |

- The system instance usually runs as a Windows service and auto-starts at boot;
  operate with `pm-sysd-ctl --system`.
- The user instance auto-starts after each user logs in; operate with
  `pm-sysd-ctl --user`.
- A custom instance is started manually; operate with
  `pm-sysd-ctl --machine=XXX`.

pm-sysd-init startup arguments (command line):

- `--system`: run a system instance.
- `--user`: run the current-user instance.
- `--machine=XXX`: run a user-custom-named instance.
- `--conf=PATH`: specify the main config file (pm-tc TOML) path (required).

Exactly one of `--system` / `--user` / `--machine=XXX` is required at startup.

### Database

pm-sysd-init uses an SQLite database to store persistent state (for example,
whether a service is enabled or masked). The database format is an internal API
and is not provided as a stable external API.

The default database file is named `pm_sysd_w_v1.db`, next to the main config
file: if the main config is `dir/XXX.toml`, the database file is
`dir/pm_sysd_w_v1.db`.

The database file path can be changed in the main config file, for example:

```toml
[env]
SYSD_DB = "path/to/XXX.db"
```

`SYSD_DB` is an absolute path; a relative path is resolved relative to the main
config file's directory.

pm-sysd-init reads `SYSD_DB` and opens the database only once at startup. A
later `daemon-reload` does not switch to a different database file (path); it
keeps using the database opened at startup.

Log data (the `logd` module) is also stored in this SQLite database. See
[db.md](db.md) for the schema.

## pm-sysd-ctl

Mimics the systemctl style (including output format), but does not use a pager;
the style is only similar, not identical. Do not include the word `systemd`,
because this project is not systemd.

Supported commands (initial version, minimal implementation):

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

soft-reboot  # restart pm-sysd-init itself and all managed services (not a Windows reboot)
exit  # exit pm-sysd-init
```

All these commands must exist in the CLI, but the initial version does not
necessarily implement them all; some may just print `TODO`. No need to support
more arguments/options either.

Supported options (meanings mostly match or resemble systemd, in the mimic
style):

```sh
-h --help

# --version is handled by pm-bin, so pm-sysd-w needs no special handling
#--version

--system
--user

-M --machine=NAME  # select the pm-sysd-w instance

--wait
--no-block
```

## pm-sysd-log

Mimics the journalctl style (including output format), but does not use a pager;
the style is only similar, not identical.

The initial version mainly implements `-eu UNIT`: jump to the end (`-e`) and
filter by unit (`-u`), printing the most recent entries. Logs are written by the
`logd` module into the SQLite database and read via init.

Supported options:

```sh
-h --help

-e  # jump to the end (default behavior)

-u --unit=UNIT  # show logs for the specified unit only

-x --catalog  # show explanatory text (no effect in the initial version, reserved)

--system
--user

-M --machine=NAME  # select the pm-sysd-w instance
```

## Supported Targets

| Target           | Description                                                               |
| ---------------- | ------------------------------------------------------------------------- |
| `default.target` | The default target after pm-sysd-init starts                              |
| `timers.target`  | (Reserved) for timers                                                     |
| `exit.target`    | When pm-sysd-init exits (conflicts with all services, thus stopping them) |

User-defined targets are currently not supported.

## Initial Scope

- Only the `service` unit type is supported; timers and other unit types are not
  supported.
- Dependencies between units are not supported.
- Custom targets are not supported.

## Project Conventions

- Source and doc comments are written in Chinese (with spaces); identifiers and
  config keywords are in English.
- No `unsafe` code (`#![deny(unsafe_code)]`).
- Error handling: each module defines its own error enum; the top-level error
  type aggregates them via `From`. Error display is best-effort, for
  development/debugging.
- `pm-bin` provides the CLI scaffolding (same as pm-tc).
- Leftover `TODO` markers indicate unfinished work and are normal.

## Code (Doc) Formatting

```sh
cargo fmt  # Rust code
deno fmt  # other files (e.g. markdown)
```

## Build & Test

```sh
cargo build
cargo test
cargo doc
cargo build --release
```
