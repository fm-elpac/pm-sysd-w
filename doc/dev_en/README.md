# pm-sysd-w Developer Documentation

## Reading Guide

This document targets developers and records development / internal content not
covered by the user guide (`doc/user/`).

- Developers must read the user guide first; this document supplements it with
  implementation details and does not repeat content already in the user guide.

- Parts that match (or closely resemble) systemd are simplified or omitted here;
  the focus is on this project's **differences** from systemd.

## Project Positioning

pm-sysd-w (PM Sysd (Windows)) mimics the style and user experience of systemd,
managing processes on Windows.

It is not systemd-compatible and implements only a small subset of systemd.
Differences from systemd and user-visible behavior are in the user guide.

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

## Binaries and Features

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

| Module       | Description                                                                                |
| ------------ | ------------------------------------------------------------------------------------------ |
| `cli`        | Common module for CLI programs (cli feature only)                                          |
| `cli::init`  | Implements the `pm-sysd-init` binary (CLI): argument parsing / help, soft-reboot execution |
| `cli::ctl`   | Implements the `pm-sysd-ctl` binary (CLI)                                                  |
| `cli::log`   | Implements the `pm-sysd-log` binary (CLI)                                                  |
| `jsonl`      | JSONL communication protocol between init and ctl                                          |
| `conf`       | Calls pm-tc to read the TOML config file                                                   |
| `db`         | SQLite database abstraction / wrapper                                                      |
| `sysd`       | Core service-management logic                                                              |
| `sysd::init` | init main loop (pm-sysd-init resident logic)                                               |
| `sysu`       | systemd-style state definitions / print output (mimicking systemd)                         |
| `sysu::ctl`  | systemctl-style output and exit codes for pm-sysd-ctl                                      |
| `sysu::log`  | journalctl-style output for pm-sysd-log                                                    |
| `logd`       | Core logging logic (journald-like, simplified)                                             |
| `pa`         | Low-level platform (OS) abstraction interface                                              |
| `p_w`        | `pa` implementation for Windows (main platform)                                            |
| `p_u`        | `pa` implementation for Linux (testing only)                                               |

## Config File (depends on pm-tc)

pm-sysd-w uses a centralized config read by pm-tc (PM Template Config Tool),
unlike systemd's many unit files.

After loading the config, pm-sysd-w calls pm-tc's `task::plan("sysd")` and gets
all unit configs from the returned `PlanDebug`. Each unit corresponds to one
`[[t.sysd]]` task instance in the config file, with unit settings passed via
`env:XXX` fields. The config fields and an example are in the user guide
(`doc/user/config.md`).

For unlisted `env:XXX` fields: fields starting with `Unit.` / `Service.` /
`Install.` / `Timer.` / `Win.` that are not supported are collected by the
`conf` module as warnings; both init startup config loading and `daemon-reload`
emit them via `log::warn!()` and write them to the log (viewable with
`pm-sysd-log`), then ignore them. All other fields are silently ignored.

## Communication (IPC)

How `pm-sysd-init` and `pm-sysd-ctl` / `pm-sysd-log` communicate:

- Linux: a UNIX socket in the current directory (testing only; default
  permissions, no special handling needed).

- Windows: a named pipe, requiring strict access control; only local programs
  may connect, network connections are forbidden.

### Wire Protocol (JSONL)

Data transfer is JSONL-based (one JSON object per line), using `serde_json` for
encoding / decoding.

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

In later messages, the `SdReq` sent by ctl / log to init is a two-level
structure: the outer `req` field (`sys-d` / `log-ro` / `unitd-ro` / `unit-d` /
`unit-ro` / `unit`) indicates the handling component and whether it is
read-only, and the `op` field in the inner `data` object indicates the specific
operation. Read-only queries and mutating operations are distinguished directly
in the data structure. For example:

```json
{ "req": "unit", "data": { "name": "a.service", "op": "start" } }
```

When several ctl commands call the same init interface, they map to a single
`op`; the output semantics are the responsibility of ctl.

### Memory Target

pm-sysd-init's expected runtime memory (maximum) is 100 MB. That is, with only
100 MB of available memory, pm-sysd-init should still run normally and not
crash.

Therefore, various defensive limits are introduced in the code to avoid using
too much memory.

### Defensive Limits

pm-sysd-init applies the following defenses to IPC connections (constants are
defined in the `pa` / `sysd::init` modules for easy adjustment):

- A single message is at most 10 MB (`pa::MAX_MESSAGE_LEN`); exceeding it
  reports an error (`log::error!()`) and closes the connection.

- A single message send must complete within 1 second (`pa::SEND_TIMEOUT`); a
  timeout reports an error and closes the connection.

- The receive timeout (`pa::RECV_TIMEOUT`) starts when the first byte of a
  message arrives and ends when the last byte of that message arrives; if the
  peer sends nothing at all, there is no timeout. A timeout reports an error and
  closes the connection.

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

The instance concept and user-visible behavior are in the user guide; this
section only records the internal implementation.

Multiple pm-sysd-init instances may run on one Windows system simultaneously.
Named pipes are used for instance uniqueness detection (a same-named instance
can only have 1 process; starting a duplicate reports an error and exits).

Named pipe name prefix: `pm_sysd_w_`.

| Instance           | Named pipe                                   | Permission                            |
| ------------------ | -------------------------------------------- | ------------------------------------- |
| System             | `pm_sysd_w_system_default`                   | Administrators only                   |
| System (read-only) | `pm_sysd_w_system_default_ro`                | Local normal users: read-only queries |
| User               | `pm_sysd_w_user_NAME` (NAME is the username) | Current user only                     |
| Custom             | `pm_sysd_w_m_XXX`                            | Current user only                     |

pm-sysd-init startup arguments (command line) and their validation rules are in
the user guide.

### Database

pm-sysd-init uses a SQLite database to store persistent state (for example,
whether a unit is enabled or masked) and logs. The database format is an
internal API and is not provided as a stable external API. The database file
location and the `SYSD_DB` config are in the user guide; the schema is in
[db.md](db.md).

All database access goes through the `db` module: it starts 2 dedicated
(synchronous) threads (a database read/write thread and a log writer thread),
and only those two threads touch SQLite directly. Tokio async threads send
requests to the database read/write thread over MPSC; each request carries a
oneshot reply, and the read/write thread sends the result back after finishing.
Async threads therefore never perform SQLite I/O directly and cannot be blocked
by slow database I/O.

### Startup Flow

pm-sysd-init starts in this order:

0. Parse / handle command-line arguments.

1. Instance uniqueness check (before all other operations); on failure, exit
   directly. On success, init already holds the UNIX socket / named pipe,
   preventing TOCTOU. On Windows, set the named-pipe ACL as documented; a
   `--system` instance also creates the `_ro` read-only named pipe (also part of
   the uniqueness check; on failure, exit as well). After the uniqueness check
   and before continuing startup, call pm-bin's `print_version()` to print the
   version info.

2. Load the config file (via the `pm-sysd-init --reload` subprocess). If loading
   takes longer than 1 s, a warning is logged and init keeps waiting; after 2 s
   the subprocess is killed, treated as a failure, and init exits directly.

3. Open the database.

4. Remaining follow-up (create SysD, start the services in the default target,
   and on success print `[  OK  ] Reached target default.target.`, then enter
   the IPC main loop).

### Internal Mechanism: Multi-actor Architecture

pm-sysd-init's `sysd` module uses a multi-actor architecture. Each key component
is a `tokio::spawn` async task holding private state, receiving and processing
messages single-threaded (async). Components communicate via MPSC messages.

The key components (`SysD` / `UnitD` / `ServiceD` / `ServiceS` / `Logd`) are
cloneable handle structs: `spawn()` starts the corresponding task (`Logd` uses
`Logd::new`).

Most components send messages non-blockingly via `send()`, waiting only for
delivery, not for processing / a reply.

Naming convention for messages and state machines (`M` suffix = message, `SM`
suffix = state machine):

- `SysD` (`sysd::sys_d`): the top-level entity, responsible only for command
  routing.

  It handles only `SdReq::SysD` (exit / soft-reboot / sqlite-vacuum).
  `SdReq::LogRO` (log queries) is forwarded to Logd, and the remaining ctl
  requests and `StartDefaultTarget` are forwarded to UnitD as-is.

  Forwarded messages do not block the main loop: each message carries its own
  oneshot reply and is forwarded directly; `sqlite-vacuum` is sent to the
  database read/write thread by a dedicated task, so it also does not block the
  main loop. The loop only waits when handling exit / soft-reboot in
  `SdReq::SysD`.

  On exit it first tells UnitD to stop all units, then notifies `sysd::init` via
  `exit_tx`, then exits its own loop.

- `UnitD` (`sysd::unit_d`): manages all units (in-memory config) and starts
  ServiceD.

  Handles daemon-reload, ctl requests, and service lifecycle orchestration.

  The `UnitDSM` state machine (`sysd::unit_d::actor`) holds the in-memory
  database `UnitS` (`sysd::unit_d::unit_s`) and is responsible for deciding and
  outputting actions.

  Config / mask-enable state reading lives in the shared `load` module
  (`sysd::init::load`), used by both pm-sysd-init startup and daemon-reload;
  enable/mask persistence lives in the actor (`sysd::unit_d::actor`), and query
  combining tasks live in `query` (`sysd::unit_d::query`).

- `ServiceD` (`sysd::service_d`): manages all services.

  State lives in `Sm` (`sysd::service_d::sm`), the actor
  (`sysd::service_d::actor`) only executes the IO requests produced by the state
  machine, and query logic lives in `query` (`sysd::service_d::query`).

  ServiceS is created / started directly by the ServiceD actor: the actor first
  records the handle and active state synchronously, then delivers the initial
  command. ServiceS does not exit by itself; when it needs to exit it sends an
  exit request to ServiceD, and ServiceD replies with `ServiceSM::Exit`.

- `ServiceS` (`sysd::service_s`): manages a single service.

  State lives in `ServiceSSM` (`sysd::service_s::actor`). ServiceS only holds
  the SM plus references to other components, not state directly.

  Its public API is `spawn()` / `send()`, and each `ServiceSM` message carries
  its own oneshot reply.

- `Logd` (`logd`): the log recorder.

  Log query requests (`SdReq::LogRO`) are handled and replied to by a background
  task; write / test queries (`write_with` / `page_entries`) use the database
  client directly.

- `TimerD` / `TimerS` (`sysd::timer_d` / `timer_s`): reserved.

---

The messages that may be sent between components (such as `SysDM` / `UnitDM` /
`ServiceDM` / `ServiceSM`, and Logd's log query requests) are each module's
public API; other components construct the message data directly and send it.

ctl requests (the `SdReq*` types in the `jsonl` module) are forwarded as whole
values where possible instead of re-wrapping every enum variant.

`SysD` / `UnitD` / `ServiceD` / `ServiceS` messages carry their own oneshot
reply, and SysD / UnitD forward them directly, so their own main loops are not
blocked.

### Internal Mechanism: Event-driven

Service lifecycle commands (start / stop / restart / reload / kill, etc.) are
read by UnitD's state machine, which forwards them to ServiceD after reading the
unit type; ServiceD replies to the caller directly. ServiceD / ServiceS each
process messages serially in a single thread. ServiceS is created / started
directly by the ServiceD actor. A single unit operation does not block the UnitD
main loop.

Service management is event-driven: a child process exit is reported by a
watcher task that `await`s the child exit and then sends a message. Scheduled
restarts send messages via a timer task.

A running ServiceS snapshots its unit config at start and keeps it unchanged for
its whole lifetime, so after `daemon-reload` a running service keeps using the
old config — no unit GC mechanism is needed.

### Graceful Exit and soft-reboot

- Exit / soft-reboot is decided by SysD: after receiving `SdReq::SysD` (or an
  exit command converted from an external signal), it first has UnitD stop all
  services; after stopping them successfully, it prints
  `[  OK  ] Reached target exit.target.`, then notifies init via `exit_tx`.
  External signals (SIGTERM / Ctrl+C) are also converted into an exit command
  sent to SysD.

- After exit, init records the log, waits for logs to be flushed, and then
  (before soft-reboot restarts itself) runs a vacuum on the database.

- `soft-reboot`, after all cleanup is done, restarts itself with the same
  command-line arguments: on Linux it `exec`s itself; on Windows it has already
  released the exclusive named pipe, so it starts a new `pm-sysd-init.exe` and
  then exits.

### Log Defensive Rules

- A single log is at most 500 KB (`logd::MAX_LOG_LEN`); the sender automatically
  splits over-long logs into multiple entries.

- When the sender's delivery fails (receive queue full), drop the log and emit a
  rate-limited `log::warn!()`.

- The `db` module's log writer thread also drops and warns on over-long logs
  (defensive check).

- Reading a subprocess's stdout / stderr: the read buffer is capped at the max
  log length, so over-long lines are split into multiple log entries and the
  buffer never grows without bound.

  Decoding uses `pa::TextDecoder` streaming decode: Linux always uses UTF-8;
  Windows tries UTF-8 first, then falls back to the system ACP, and only if ACP
  also fails is the line replaced with one error log entry written to the unit.

- `pm-sysd-log`'s log-reading interface is a `pagination` interface returning
  total count / cursor.

  `logd` first selects the page with an estimate, then verifies with an actual
  encode, keeping each page within the jsonl 10 MB single-message limit. The
  database read side also stops early based on the cumulative log content length
  to avoid reading too much at once. `pm-sysd-log` reads repeatedly via the
  cursor until it has collected the requested number.

- A single paginated query returns at most 10000 entries (`logd::LOG_PAGE_MAX`);
  larger external requests are automatically capped.

- Log retention: automatic cleanup runs once per hour, keeping only the last 72
  hours (constants in the `logd` module).

### Named-pipe permission test (manual)

`test/test-permission.ps1` manually verifies the named-pipe ACL permissions on
Windows. It must run as administrator, and the binaries must be built first:

```cmd
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

## pm-sysd-ctl / pm-sysd-log

The command list and options are in the user guide. This section only records
the internal implementation.

Mimics the systemctl / journalctl style (including output format), but does not
use a pager; the style is only similar, not identical. The output must not
contain words such as `systemd`, because this project is not systemd.

All commands must exist in the CLI, but the initial version does not necessarily
implement them all; some may just print `TODO`. No need to support more
arguments / options either.

- `list-units` is a paginated interface: the request carries `limit` and a
  cursor (the last unit name), and the response carries the total, the next-page
  cursor, and the current page of units.

  UnitD combines the current config with the services still running in ServiceD
  (including masked services that are still running, which are shown as masked),
  sorted by unit name.

  Before returning, it actually JSON-encodes the whole response; if it exceeds 1
  MB, it reduces the page size by halving (keeping at least one entry), so the
  response stays within the jsonl single-message limit.

  `pm-sysd-ctl` automatically fetches all pages and then prints them.

### Connection failure (fast fail)

`pm-sysd-ctl` / `pm-sysd-log` first check whether the IPC endpoint (named pipe /
UNIX socket) exists, and only attempt to connect after it exists, instead of
connecting directly. Connection retries take at most 200ms total, so a missing
init fails quickly instead of keeping the user waiting.

When connecting to `--system` on Windows, the main pipe
`pm_sysd_w_system_default` is tried first; if that fails due to insufficient
permissions, the read-only pipe `pm_sysd_w_system_default_ro` is tried next
(ordinary users can only make read-only queries).

The default instance selection (`--system` first, fall back to `--user`) and its
rationale are in the user guide.

## install/setup.ps1 (Install Script)

`install/setup.ps1` is an experimental install / uninstall script providing two
commands, `install-user` and `uninstall-user`. It uses PowerShell to call
Windows tools directly and does not depend on pm-sysd-w's own Rust code.

Administrator privileges are required (installing / removing a service writes to
`HKLM`).

### install-user

Runs in order:

1. Copies `pm-sysd-init.exe` / `pm-sysd-ctl.exe` / `pm-sysd-log.exe`. Lookup
   order: the current directory, its `target/release/`, the script's parent
   directory, and its `target/release/`. They are copied to
   `%LOCALAPPDATA%\pm-sysd-w\bin\`.

2. Reads the current user's `PATH` with
   `[Environment]::GetEnvironmentVariable('Path', 'User')`; if the target
   directory is missing, appends it and writes it back with
   `[Environment]::SetEnvironmentVariable('Path', ..., 'User')`.

3. If `%LOCALAPPDATA%\pm-sysd-w\pm_sysd_w.conf.toml` does not exist, writes the
   empty config `pm_tc = 1` with `[System.IO.File]::WriteAllText`.
   `conf::load_conf` allows a config without any `[[t.sysd]]`, so the file loads
   normally.

4. Registers `pm-sysd-init.exe --user --conf=...` as a per-user service template
   using `sc.exe create pm_sysd_w_user type= userown start= auto binPath= ...`.
   Its `Type` is `SERVICE_USER_OWN_PROCESS` (0x50), and the template lives at
   `HKLM\SYSTEM\CurrentControlSet\Services\pm_sysd_w_user`.

   Per-user services are managed by Windows SCM: when a user signs in, SCM
   creates the `pm_sysd_w_user_<LUID>` instance from the template and starts it;
   when the user signs out, the instance is stopped and deleted.

### uninstall-user

`sc.exe delete pm_sysd_w_user` removes the service template and leaves the
`%LOCALAPPDATA%\pm-sysd-w\` directory untouched.

### pm-sysd-init Windows Service Integration

On startup (Windows only), `pm-sysd-init` first tries to connect to SCM
(`StartServiceCtrlDispatcherW`): success means it was started by SCM and enters
service mode; `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` means it was started
from the command line, so it continues with the normal flow. The service mode is
implemented in `p_w::service`:

- The service main registers a control handler and reports
  `SERVICE_START_PENDING` / `SERVICE_RUNNING` etc., so SCM does not terminate
  the service with "did not respond in a timely fashion" (error 1053).
- On `SERVICE_CONTROL_STOP` / `SERVICE_CONTROL_SHUTDOWN` it sets a stop event;
  `pa::spawn_signal_watch` waits for that event in service mode, triggering the
  same graceful exit path as Ctrl+C.
- The service command line must specify `--conf` explicitly (no default is
  applied).

### Known Limitations

- The service template is machine-level (HKLM) config, but the script places the
  binaries in the current user's `%LOCALAPPDATA%`. Therefore this install method
  currently suits only single-user scenarios; other users cannot directly reuse
  the template.

- In service mode, `soft-reboot` does not restart the process itself (it is
  treated as a normal exit), so the process does not leave SCM management.

## Project Conventions

For specific requirements on writing code, see `AGENTS.md` (`Code Conventions`)
and the comments in the source modules; they are not repeated here. This is
because the project is mainly written by AI, while humans mainly do architecture
design and code review.

- `pm-bin` provides the CLI scaffolding (same as pm-tc).

### Code (Doc) Formatting

```sh
cargo fmt  # Rust code
deno fmt  # other files (e.g. markdown)
```

## Build & Test

See `AGENTS.md` (the `Common Commands` section) and CI
(`.github/workflows/ci.yml`).
