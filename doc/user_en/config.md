# pm-sysd-w Config Reference

pm-sysd-w uses pm-tc's centralized TOML format, with multiple units in one file.
This page describes the config fields supported by the initial (simple)
implementation. It is a draft and may change.

## Top Level

```toml
pm_tc = 1
```

`pm_tc = 1` is required and denotes the pm-tc config format version.

The `[env]` section (optional) holds pm-sysd-w's own settings:

| Field     | Type   | Required | Default           | Description                                                                                          |
| --------- | ------ | -------- | ----------------- | ---------------------------------------------------------------------------------------------------- |
| `SYSD_DB` | string | no       | `pm_sysd_w_v1.db` | SQLite DB path (relative to the main config dir); read only at startup, not changed by daemon-reload |

## unit (service)

Each `[[t.sysd]]` defines one unit (the initial version supports only the
`service` type). Settings are passed via `env:XXX` fields, similar to systemd
unit file section/key names.

Boolean fields support the systemd boolean syntax: `1` / `yes` / `true` / `on`
mean true, and `0` / `no` / `false` / `off` mean false.

### `env:Unit.*` (general)

| Field                  | Type   | Required | Default | Description                         |
| ---------------------- | ------ | -------- | ------- | ----------------------------------- |
| `env:Unit`             | string | yes      | -       | Unit name; must end with `.service` |
| `env:Unit.Description` | string | no       | empty   | Description                         |

### `env:Service.*`

| Field                          | Type       | Required | Default           | Description                                              |
| ------------------------------ | ---------- | -------- | ----------------- | -------------------------------------------------------- |
| `env:Service.Type`             | enum       | no       | `simple`          | Start type (values below)                                |
| `env:Service.ExecStart`        | string     | yes      | -                 | Start command (program path + args)                      |
| `env:Service.ExecStop`         | string     | no       | -                 | Stop command                                             |
| `env:Service.ExecReload`       | string     | no       | -                 | Reload command                                           |
| `env:Service.Restart`          | enum       | no       | `no`              | Restart policy (values below)                            |
| `env:Service.RestartSec`       | nonneg int | no       | `1`               | Seconds to wait before restart (only when Restart != no) |
| `env:Service.TimeoutStartSec`  | nonneg int | no       | `90`              | Start timeout (seconds)                                  |
| `env:Service.TimeoutStopSec`   | nonneg int | no       | `90`              | Stop timeout (seconds)                                   |
| `env:Service.WorkingDirectory` | string     | no       | inherit from init | Working directory                                        |
| `env:Service.Environment`      | string     | no       | -                 | Environment variable, `KEY=VALUE`                        |
| `env:Service.RemainAfterExit`  | bool       | no       | `no`              | Whether to stay active after the process exits           |

`Type` values (initial version):

| Value     | Description                                                                     |
| --------- | ------------------------------------------------------------------------------- |
| `simple`  | Default; considered active as soon as it is started                             |
| `oneshot` | Considered started when the process exits (successfully); does not keep running |

`Restart` values (initial simple implementation):

| Value        | Description                                    |
| ------------ | ---------------------------------------------- |
| `no`         | Do not auto-restart (default)                  |
| `always`     | Always restart after exit                      |
| `on-failure` | Restart only on abnormal exit (nonzero/signal) |

### `env:Install.*`

| Field                  | Type   | Required | Default | Description                                                             |
| ---------------------- | ------ | -------- | ------- | ----------------------------------------------------------------------- |
| `env:Install.WantedBy` | string | no       | empty   | Target the unit belongs to (built-in only); must be set to allow enable |

### `env:Win.*` (pm-sysd-w specific)

| Field                    | Type | Required | Default | Description                                                                                                              |
| ------------------------ | ---- | -------- | ------- | ------------------------------------------------------------------------------------------------------------------------ |
| `env:Win.CreateNoWindow` | bool | no       | `no`    | Set CREATE_NO_WINDOW when starting the process, so console programs show no black window; Windows only, ignored on Linux |

## Built-in Targets

Only built-in targets are supported in the initial version; custom targets are
not:

| Target           | Description                                   |
| ---------------- | --------------------------------------------- |
| `default.target` | Default target after init starts              |
| `timers.target`  | (Reserved) for timers                         |
| `exit.target`    | When init exits (conflicts with all services) |

## Example

See `example/pm_sysd_w.conf.toml`.

## Initial Scope (Not Supported)

- Only the `service` unit type; timers and other types are not supported.
- Dependencies between units are not supported.
- Custom targets are not supported.
- Fields not listed are not implemented in the initial version (they may be
  ignored or kept).
