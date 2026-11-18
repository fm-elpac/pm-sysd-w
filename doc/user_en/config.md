# pm-sysd-w Config Reference

pm-sysd-w uses pm-tc's centralized TOML format, with multiple units in one file.
This page describes the config fields supported by the initial (simple)
implementation.

## Top Level

```toml
pm_tc = 1
```

- `pm_tc = 1` is required and denotes the pm-tc config format version.

- pm-tc's `[[include]]` feature is supported: the main config file can be split
  into multiple TOML files and referenced.

- The `[env]` section (optional) holds pm-sysd-w's own settings:

| Field     | Type   | Required | Default           | Description                                       |
| --------- | ------ | -------- | ----------------- | ------------------------------------------------- |
| `SYSD_DB` | string | no       | `pm_sysd_w_v1.db` | Local database path (relative to the main config) |

- `SYSD_DB` is read only at startup; `daemon-reload` does not change it.

## unit (service)

Each `[[t.sysd]]` defines one unit (the initial version supports only the
`service` type). Settings are passed via `env:XXX` fields, similar to systemd
unit file section/key names.

- Boolean fields support the systemd boolean syntax: `1` / `yes` / `on` / `true`
  mean true, and `0` / `no` / `off` / `false` mean false.

### `env:Unit.*` (general)

| Field                            | Type       | Required | Default | Description                                     |
| -------------------------------- | ---------- | -------- | ------- | ----------------------------------------------- |
| `env:Unit`                       | string     | yes      | -       | Unit name; currently only the `.service` suffix |
| `env:Unit.Description`           | string     | no       | empty   | Description                                     |
| `env:Unit.StartLimitIntervalSec` | nonneg int | no       | `10`    | Start rate-limit window (seconds); `0` disables |
| `env:Unit.StartLimitBurst`       | nonneg int | no       | `5`     | Start rate-limit burst; `0` disables            |

- Exceeding the burst within the `start rate-limit window` rejects starts /
  stops auto-restarting.

### `env:Service.*`

| Field                          | Type       | Required | Default           | Description                                                       |
| ------------------------------ | ---------- | -------- | ----------------- | ----------------------------------------------------------------- |
| `env:Service.Type`             | enum       | no       | `simple`          | Start type (values below)                                         |
| `env:Service.ExecStart`        | string     | no       | -                 | Start command (program path + args); supports `multi-line syntax` |
| `env:Service.ExecStop`         | string     | no       | -                 | Stop command; supports `multi-line syntax`                        |
| `env:Service.ExecReload`       | string     | no       | -                 | Reload command; supports `multi-line syntax`                      |
| `env:Service.Restart`          | enum       | no       | `no`              | Restart policy (values below)                                     |
| `env:Service.RestartSec`       | nonneg int | no       | `1`               | Seconds to wait before restart; only when Restart != no           |
| `env:Service.TimeoutStartSec`  | nonneg int | no       | `90`              | Start timeout (seconds); only for `Type=oneshot`, `0` disables    |
| `env:Service.TimeoutStopSec`   | nonneg int | no       | `90`              | Stop timeout (seconds); `0` disables                              |
| `env:Service.WorkingDirectory` | string     | no       | inherit from init | Working directory                                                 |
| `env:Service.Environment`      | string     | no       | -                 | Environment variable, `NAME=VALUE`; supports `multi-line syntax`  |
| `env:Service.RemainAfterExit`  | bool       | no       | `no`              | Whether to stay active after the process exits                    |

`Type` values:

| Value     | Description                                                                                                            |
| --------- | ---------------------------------------------------------------------------------------------------------------------- |
| `simple`  | Default; considered active as soon as it is started                                                                    |
| `oneshot` | Considered started when the process exits (successfully); does not keep running; `start` waits for ExecStart to finish |

`Restart` values:

| Value        | Description                                    |
| ------------ | ---------------------------------------------- |
| `no`         | Do not auto-restart (default)                  |
| `always`     | Always restart after exit                      |
| `on-failure` | Restart only on abnormal exit (nonzero/signal) |

---

`Exec*` notes:

- All such fields (`ExecStart`, `ExecStop`, `ExecReload`) use pm-tc to split the
  command-line arguments (equivalent to `pm-tc --exec-eval`). Shell quoting /
  escaping syntax is not supported.

---

`ExecStart` and `ExecStop` requirements: the overall principle is that the
service must be meaningful. Specific rules:

- `ExecStart` and `ExecStop` cannot both be empty, because that would mean the
  service does nothing.

- If `Type=simple`, then `ExecStart` is required, because `simple` means running
  something continuously, so there must be something to run.

- If `ExecStart` is empty, `RemainAfterExit=yes` must also be set; then the
  service succeeds immediately when started. Otherwise, if `RemainAfterExit=no`,
  it would do nothing.

This check is implemented.

---

`Environment` value: one `env:Service.Environment` sets a single environment
variable. To set multiple environment variables, use `multi-line syntax`.

### `env:Install.*`

| Field                  | Type   | Required | Default | Description                                                                     |
| ---------------------- | ------ | -------- | ------- | ------------------------------------------------------------------------------- |
| `env:Install.WantedBy` | string | no       | empty   | Target the unit belongs to; only `default.target`; supports `multi-line syntax` |

### `env:Win.*` (Windows-specific extension)

| Field                    | Type | Required | Default | Description                                                                                |
| ------------------------ | ---- | -------- | ------- | ------------------------------------------------------------------------------------------ |
| `env:Win.CreateNoWindow` | bool | no       | `no`    | Set `CREATE_NO_WINDOW` when starting the process, so console programs show no black window |

Windows only; ignored on Linux.

## Multi-line Syntax

The `conf` module implements parsing / checking of multi-line syntax. Currently
the `sysd` runtime only uses the first `ExecStart` / `ExecStop` / `ExecReload`
command.

Because TOML does not support duplicate keys, this `multi-line syntax` is
introduced to support writing several (multi-line) `ExecStart=` or
`Environment=` entries in a row, similar to a systemd unit file.

`multi-line syntax` means executing multiple commands in sequence, or a setting
with multiple values (for example, specifying multiple environment variables).

For example:

| Key syntax                | Prefix              | Suffix | Remark       |
| :------------------------ | :------------------ | :----- | :----------- |
| `env:Service.ExecStart`   | `Service.ExecStart` | ``     | empty string |
| `env:Service.ExecStart.1` | `Service.ExecStart` | `1`    |              |
| `env:Service.ExecStart.2` | `Service.ExecStart` | `2`    |              |
| `env:Service.ExecStart.a` | `Service.ExecStart` | `a`    |              |
| `env:Service.ExecStart.b` | `Service.ExecStart` | `b`    |              |
| `env:Service.ExecStart.c` | `Service.ExecStart` | `c`    |              |

- Key syntax: how it is actually written in the TOML config file.

- Prefix: a pm-sysd-w supported config item, described earlier in this document.

- Suffix: the user-defined part, separated from the prefix by `.` (everything
  after the prefix is the suffix, and the suffix itself may contain `.`).

  Suffixes are sorted in `ascending string` order (empty / shorter strings come
  first).

  Prefer unambiguous suffixes to avoid surprising ordering. For example, up to
  10 entries, use `.1`, `.2`; up to 100 entries, use `.01`, `.02`.

Note: `env:Service.ExecStart.` is not supported (error), because the suffix is
still the empty string; use `env:Service.ExecStart` instead.

## Built-in Targets

Only built-in targets are supported; custom targets are not:

| Target           | Description                                |
| ---------------- | ------------------------------------------ |
| `default.target` | Default target after init starts           |
| `timers.target`  | (Reserved) for timers                      |
| `exit.target`    | When init exits (conflicts with all units) |

## Validation Limits

- The total size of all TOML files loaded by pm-tc must not exceed 10 MB;
  exceeding it fails immediately.

- A unit name must be at most 500 bytes (UTF-8) and must pass pm-tc's
  variable-name check. The name must end with a `.service` / `.timer` /
  `.target` suffix, and must not be empty before the suffix. The initial version
  still implements only the `service` type, so `.timer` / `.target` names are
  rejected.

- For a single supported `env:` key-value pair, the key is at most 500 bytes
  (UTF-8) and the value is at most 20 KB (UTF-8).

  The total of all supported key-value pairs in one unit must not exceed 500 KB.
  Unsupported keys are not counted.

## Example

See `example/pm_sysd_w.conf.toml`.

## Initial Scope (Not Supported)

- Dependencies between units are not supported.

- Custom `target`s are not supported.

- systemd `template unit` usage is not supported (for example the
  `NAME@.service` form).

- Unlisted fields are not implemented in the initial version.

  Unlisted fields starting with `Unit.` / `Service.` / `Install.` / `Timer.` /
  `Win.` write a warning log entry (viewable with `pm-sysd-log`) both when init
  loads the config at startup and on `daemon-reload`, then are ignored.

  All other fields are silently ignored.
