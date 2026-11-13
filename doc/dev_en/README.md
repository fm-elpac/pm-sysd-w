# pm-sysd-w Developer Documentation

pm-sysd-w (PM Sysd (Windows)) is a systemd-style process manager for Windows.

It mimics the style and user experience of systemd, managing processes
(services) on Windows; it is not systemd-compatible and implements only a small
subset of systemd.

The name "稳哒" (Wěndā) comes from the internet phrase "稳稳哒接住你" ("catch
you steadily").

## Build Targets

pm-sysd-w supports 2 build targets:

| Target  | Triple                     | Purpose                     |
| ------- | -------------------------- | --------------------------- |
| Windows | `x86_64-pc-windows-msvc`   | Main supported platform     |
| Linux   | `x86_64-unknown-linux-gnu` | For convenient testing only |

Only Windows 10 or newer is supported.

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

Note: `service` in this documentation is not the Windows service concept. It is
a pm-sysd-w unit type, similar to systemd's service concept; in practice it is
one (or a group of) process(es) started by `pm-sysd-init`.

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

| Module       | Description                                                                                |
| ------------ | ------------------------------------------------------------------------------------------ |
| `cli`        | Common module for CLI programs (cli feature only)                                          |
| `cli::init`  | Implements the `pm-sysd-init` binary (CLI): argument parsing / help, soft-reboot execution |
| `cli::ctl`   | Implements the `pm-sysd-ctl` binary (CLI)                                                  |
| `cli::log`   | Implements the `pm-sysd-log` binary (CLI)                                                  |
| `jsonl`      | JSONL communication protocol between init and ctl                                          |
| `conf`       | Calls pm-tc to read the TOML config file                                                   |
| `db`         | SQLite database abstraction/wrapper                                                        |
| `sysd`       | Core service-management logic                                                              |
| `sysd::init` | init main loop (pm-sysd-init resident logic)                                               |
| `logd`       | Core logging logic (journald-like, simplified)                                             |
| `pa`         | Low-level platform (OS) abstraction interface                                              |
| `p_w`        | `pa` implementation for Windows (main platform)                                            |
| `p_u`        | `pa` implementation for Linux (testing only)                                               |

## Config File (depends on pm-tc)

Instead of systemd's "many config files" approach, pm-sysd-w uses a
**centralized** config: many units are written in one TOML file, read by pm-tc
(PM Template Config Tool).

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

For unlisted `env:XXX` fields: fields starting with `Unit.` / `Service.` /
`Install.` / `Timer.` / `Win.` that are not supported are collected by the
`conf` module as warnings; both init startup config loading and `daemon-reload`
emit them via `log::warn!()` and write them to the log (viewable with
pm-sysd-log), then ignore them. All other fields are silently ignored.

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
   reports an error. On a version mismatch, init first sends its own version
   (handshake message) to the peer, then closes the connection.

Only the handshake message carries a version number; later messages do not need
one.

### Defensive Limits

pm-sysd-init applies the following defenses to IPC connections (constants are
defined in the `jsonl` / `sysd::init` modules for easy adjustment):

- A single message is at most 1 MB (`jsonl::MAX_MESSAGE_LEN`); exceeding it
  reports an error (`log::error!()`) and closes the connection.
- A single message send / receive must complete within 1 second
  (`jsonl::RECV_TIMEOUT` / `jsonl::SEND_TIMEOUT`); a timeout reports an error
  and closes the connection.
- pm-sysd-init allows at most 200 concurrent connections
  (`sysd::init::MAX_CONNECTIONS`); exceeding it reports an error and closes the
  connection.
- A message that fails to parse (protocol-level mismatched data): report an
  error and close the connection.
- A handshake message that cannot be parsed: report an error and close the
  connection; on a version mismatch, init sends its own version then closes.
- Reads and writes use buffered chunks (`fill_buf` / `consume`, `write_all`),
  not byte-by-byte.

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

Note: the current version does not include auto-start at boot (running as a
Windows service) or auto-start after user login; these startup methods must be
configured manually (for example, installing `pm-sysd-init` as a Windows
service).

pm-sysd-init startup arguments (command line):

- `--system`: run a system instance.
- `--user`: run the current-user instance.
- `--machine=XXX`: run a user-custom-named instance.
- `--conf=PATH`: specify the main config file (pm-tc TOML) path (required).

Exactly one of `--system` / `--user` / `--machine=XXX` is required at startup,
and they cannot be combined; specifying more than one reports an error and
exits.

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

All database access goes through the `db` module: it starts 2 dedicated
(synchronous) threads (a database read/write thread and a log writer thread),
and only those two threads touch SQLite directly. Tokio async threads send
requests to the database read/write thread over MPSC; each request carries a
response MPSC, and the read/write thread sends the result back after finishing.
Async threads therefore never perform SQLite I/O directly and cannot be blocked
by slow database I/O.

### Startup Flow

pm-sysd-init starts in this order:

0. Parse / handle command-line arguments.
1. Instance uniqueness check (before all other operations); on failure, exit
   directly. On success, init already holds the UNIX socket / named pipe,
   preventing TOCTOU. On Windows, set the named-pipe ACL as documented; a
   `--system` instance also creates the `_ro` read-only named pipe (also part of
   the uniqueness check; on failure, exit as well).
2. Load the config file; on failure, exit directly.
3. Open the database.
4. Remaining follow-up (create SysD, asynchronously start enabled services in
   the default target, enter the IPC main loop).

### Internal Mechanism: Multi-actor Architecture

pm-sysd-init's `sysd` module uses a multi-actor architecture. Each key component
is a `tokio::spawn` async task holding private state and processing that state
single-threaded; components communicate via MPSC messages. The key components
(`SysD` / `UnitD` / `ServiceD` / `ServiceS` / `Logd`) are cloneable handle
structs: `spawn()` (or `Logd::new` for `Logd`) starts the corresponding task.
Most components use `req()` to send a message and await the reply; Logd's log
query requests use `req_log_ro()`, which only awaits delivery, not processing.

Naming convention for messages and state machines (M suffix = message, SM suffix
= state machine):

- `SysD` (`sysd::sys_d`): the top-level entity, responsible only for command
  routing. It handles `SdReq::SysD` (exit / soft-reboot) itself; `SdReq::LogRO`
  (log queries) is forwarded to Logd, and all other ctl requests and
  `StartDefaultTarget` are forwarded to UnitD as-is. Forwarded messages do not
  block the main loop: each message carries its own oneshot reply and is
  forwarded directly, so the loop only waits when handling `SdReq::SysD`. On
  exit it first tells UnitD to stop all units, then notifies `sysd::init` via
  `exit_tx`, then exits its own loop.
- `UnitD` (`sysd::unit_d`): manages all units (in-memory config), starts
  ServiceD, and handles `daemon-reload`, ctl requests, and service lifecycle
  orchestration. The `UnitDSM` state machine (`sysd::unit_d::actor`) holds the
  in-memory database `UnitS` (`sysd::unit_d::unit_s`) and outputs actions;
  config loading / persistence lives in `load` (`sysd::unit_d::load`), and query
  combining tasks live in `query` (`sysd::unit_d::query`).
- `ServiceD` (`sysd::service_d`): manages all services; state lives in
  `ServiceDSM` (`sysd::service_d::actor`) and query logic lives in `query`
  (`sysd::service_d::query`). ServiceS creation (new start) is delegated to a
  dedicated job-queue task that creates instances serially; the main loop only
  enqueues start jobs.
- `ServiceS` (`sysd::service_s`): manages a single service; state lives in
  `ServiceSSM` (`sysd::service_s::actor`). ServiceS only holds the SM plus
  references to other components, not state directly. Its public API is
  `spawn()` / `send()`, and each `ServiceSM` message carries its own oneshot
  reply.
- `Logd` (`logd`): the log recorder. Log query requests (`SdReq::LogRO`) are
  handled and replied to by a background task; write / test queries
  (`write_with` / `recent_entries` / `page_entries`) use the database client
  directly.
- `TimerD` / `TimerS` (`sysd::timer_d` / `timer_s`): reserved.

The messages that may be sent between components (such as `SysDM` / `UnitDM` /
`ServiceDM` / `ServiceSM`, and Logd's log query request) are each module's
public API; other components construct the message data directly and send it.
ctl requests (the `SdReq*` types in the `jsonl` module) are forwarded as whole
values where possible instead of re-wrapping every enum variant. `SysD`,
`UnitD`, `ServiceD`, and `ServiceS` messages carry their own oneshot reply, and
SysD / UnitD forward them directly, so their own main loops are not blocked.

### Internal Mechanism: Event-driven Service Management

Service lifecycle commands (start / stop / restart / reload / kill, etc.) are
forwarded by UnitD's state machine to ServiceD after reading the unit type, and
ServiceD replies to the caller directly; ServiceD and ServiceS each process
messages serially. ServiceS creation (new start) is delegated to ServiceD's
job-queue task, which creates instances serially; the main loop only enqueues
start jobs. Single-unit operations do not block the UnitD main loop.

Service management no longer polls; it is event-driven: a child watcher awaits
the child exit and sends a message, and a timer task sends a message for
scheduled restarts. A running ServiceS snapshots its unit config at start and
keeps it unchanged for its whole lifetime, so after `daemon-reload` a running
service keeps using the old config — no unit GC mechanism is needed.

### Graceful Exit and soft-reboot

- Exit / soft-reboot is decided by SysD: after receiving `SdReq::SysD` (or an
  exit command converted from an external signal), it first has UnitD stop all
  services, then notifies init via `exit_tx`. External signals (SIGTERM /
  Ctrl+C) are also converted into an exit command sent to SysD.
- After exit, init records the log and waits for logs to be flushed.
- `soft-reboot`, after all cleanup is done, restarts itself with the same
  command-line arguments: on Linux it `exec`s itself; on Windows it has already
  released the exclusive named pipe, so it starts a new `pm-sysd-init.exe` and
  then exits.

### Log Defensive Rules

- A single log is at most 500 KB (`db::MAX_LOG_LEN`); the sender automatically
  splits over-long logs into multiple entries.
- When the sender's delivery fails (receive queue full), drop the log and emit a
  rate-limited `log::warn!()`.
- The `db` module's log writer thread also drops and warns on over-long logs
  (defensive check).
- Reading a subprocess's stdout / stderr: a single line is limited by the max
  log length, and over-long lines are split into multiple log entries. stdout /
  stderr is not guaranteed to be UTF-8; on decode failure it is replaced with
  one error log entry written to the unit.
- `pm-sysd-log`'s log-reading interface is a "pagination" interface returning
  total count / cursor, for flexible reading and to fit the jsonl 1 MB
  single-message limit.
- Log retention: automatic cleanup runs once per hour, keeping only the last 72
  hours (constants in the `logd` module).

### Named-pipe permission test (manual)

`test/test-permission.ps1` manually verifies the named-pipe ACL permissions on
Windows. It must run as administrator, and the binaries must be built first:

```sh
cargo build --features cli_init
cargo build --features cli_ctl
powershell -ExecutionPolicy Bypass -File test\test-permission.ps1
```

The script starts a `--system` instance, creates a temporary standard user, and
tries connecting to the main pipe (`pm_sysd_w_system_default`) and the read-only
pipe (`pm_sysd_w_system_default_ro`) as both administrator and normal user,
checking that a normal user cannot connect to the main pipe but can connect to
the read-only pipe. This script must be run manually (automated tests do not
cover cross-user permissions).

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

### Default instance selection (differs from systemd)

When `pm-sysd-ctl` / `pm-sysd-log` do not explicitly specify an instance, they
try `--system` first; if `--system` does not exist (is not running), they fall
back to `--user`.

This differs from systemd's default behavior: `systemctl` connects only to the
system instance by default and does not fall back. pm-sysd-w adds the fallback
because, on Windows, ordinary users often have no system instance and more
commonly use the user instance.

### Connection failure (fast fail)

`pm-sysd-ctl` / `pm-sysd-log` first check whether the IPC endpoint (named pipe /
UNIX socket) exists, and only attempt to connect after it exists, instead of
connecting directly. Connection retries take at most 200ms total, so a missing
init fails quickly instead of keeping the user waiting.

When connecting to `--system` on Windows, the main pipe
`pm_sysd_w_system_default` is tried first; if that fails due to insufficient
permissions, the read-only pipe `pm_sysd_w_system_default_ro` is tried next
(ordinary users can only make read-only queries).

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

- The planned unit types are only `service` / `timer` / `target`; the current
  version implements only `service`, does not implement `timer`, and does not
  support custom `target`s.
- Dependencies between units are not supported.
- Custom targets are not supported.

## Project Conventions

- Source and doc comments are written in Chinese (with spaces); identifiers and
  config keywords are in English.
- No `unsafe` code (`#![deny(unsafe_code)]`). Exceptions: the `p_w::console` and
  `p_w::acl` modules (the Windows APIs have no safe alternative and require
  `unsafe fn` calls); they override the crate-level deny with
  `#[allow(unsafe_code)]` and keep the unsafe calls contained.
- Error handling: each module defines its own error enum; the top-level error
  type aggregates them via `From`. Error display is best-effort, for
  development/debugging.
- No silent error swallowing: generally do not discard `Result`/errors with
  `let _ =` or `.ok()`; on error use `unwrap` / `expect`, or propagate with `?`.
  Exceptions: `Drop` impls and the `unsafe`-allowed modules (`p_w::console` /
  `p_w::acl`) may silently ignore errors; every place that ignores an error or
  looks like silent swallowing (`let _ =`) must carry a comment explaining why.
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
