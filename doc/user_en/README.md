# pm-sysd-w User Guide

pm-sysd-w (PM Sysd (Windows)) manages processes on Windows in a systemd-like
way.

Use `pm-sysd-ctl` to operate services, similar to `systemctl`; use `pm-sysd-log`
to view logs, similar to `journalctl`.

pm-sysd-w is not systemd-compatible and implements only a small subset of
systemd. This guide focuses on the **differences** from systemd; parts that
match (or closely resemble) systemd are simplified or omitted.

## Document Index

| Document                 | Contents                                 |
| ------------------------ | ---------------------------------------- |
| [install.md](install.md) | Build / install / quick start            |
| [config.md](config.md)   | Config reference                         |
| [ctl.md](ctl.md)         | `pm-sysd-ctl` commands                   |
| [log.md](log.md)         | `pm-sysd-log` commands and log notes     |
| [win.md](win.md)         | Windows integration / exclusive features |
| [faq.md](faq.md)         | FAQ (empty for now)                      |

## Concepts

`service` is a pm-sysd-w unit type, similar in concept to systemd's service.
Note that it is not the Windows "Windows service"; in practice it is one (or a
group of) process(es) started and managed by `pm-sysd-init`.

`pm-sysd-init` is a resident daemon (similar to systemd's PID 1 role) that
starts and manages services. `pm-sysd-ctl` / `pm-sysd-log` communicate with it
(the instance) to perform operations.

## Instances

Multiple `pm-sysd-init` instances may run on one system simultaneously. Select
the instance to operate on with `pm-sysd-ctl` / `pm-sysd-log`'s instance option:

| Option           | Instance        | Description                                                    |
| ---------------- | --------------- | -------------------------------------------------------------- |
| `--system`       | System instance | Machine-wide, requires admin                                   |
| `--user`         | User instance   | Current logged-in user                                         |
| `--machine=NAME` | Custom instance | Started manually; NAME uses only [0-9A-Za-z_-], length 1..=100 |

- When starting, `pm-sysd-init` must specify `--conf=PATH` (the main config
  file) and choose exactly one of the instance options above.

- When `pm-sysd-ctl` / `pm-sysd-log` do not specify an instance option, they try
  `--system` first; if the system instance is not running, they automatically
  fall back to `--user`. When connecting to `--system`, ordinary
  (non-administrator) users automatically use a read-only connection. See
  [ctl.md](ctl.md).

## Key Differences from systemd

- Config file: uses pm-tc's **centralized** TOML format, with multiple units in
  one file, instead of systemd's multiple unit files.

- Deployment: by default the current version does not include auto-start at boot
  / after login. The experimental `install/setup.ps1` is available (see
  [install.md](install.md)); other auto-start methods must still be handled
  manually.

- Default instance: `pm-sysd-ctl` / `pm-sysd-log` try `--system` first and fall
  back to `--user` if it is not running (systemd connects only to the system
  instance by default and does not fall back).

- Permissions: an ordinary (non-administrator) user connecting to `--system`
  automatically uses a read-only connection and can only make read-only queries
  (such as viewing status / logs).

- The `cat` command prints JSON, not systemd's unit file content.

- `soft-reboot` restarts `pm-sysd-init` itself and all managed services, not
  Windows.

- On Windows shutdown / logoff, a graceful service stop cannot be guaranteed
  (see [win.md](win.md)).

- Logs are stored in a local database and kept for the last 72 hours by default;
  there is also a pm-sysd-w-specific command `sqlite-vacuum` for cleaning /
  optimizing the database.
