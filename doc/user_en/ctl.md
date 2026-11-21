# pm-sysd-ctl Commands

`pm-sysd-ctl` operates services, similar in style to `systemctl`. When no
instance is specified, it tries `--system` first; if the system instance is not
running, it falls back to `--user`. When connecting to `--system`, an ordinary
(non-administrator) user automatically uses a read-only connection and can only
make read-only queries.

## Command List

Supported commands (initial version); some commands may just print `TODO` in the
initial version:

```sh
list-units
list-timers

status
is-active
is-failed
show
cat
list-dependencies

start
stop
restart
try-restart
reload
reload-or-restart
try-reload-or-restart
kill
reset-failed
clean

enable
disable
is-enabled
reenable
preset
mask
unmask
get-default
set-default

daemon-reload
whoami
is-system-running

show-environment
set-environment
unset-environment
import-environment

# restart pm-sysd-init itself and all managed services
soft-reboot
# exit pm-sysd-init
exit

# clean / optimize the sqlite database
sqlite-vacuum
```

## Common Options

```sh
   --version
-h --help

   --system
   --user
-M --machine=NAME

   --wait
   --no-block
```

## Command Behavior Notes

- Unit names without a suffix get `.service` appended automatically (for example
  `status test` equals `status test.service`).

- The output format of `list-units` / `status` / `list-timers` mimics
  `systemctl` as much as possible; fields not relevant to this project are
  omitted.

- `list-units` sorts by unit name. Units removed from the new config by
  `daemon-reload` but still running are also listed (their state comes from the
  running old config).

- `enable` / `disable` support `--now` (for example `enable --now`).

- In the initial version, `preset` is equivalent to `disable`.

- `mask` behavior matches systemd: it supports `--now` (by default it only
  masks, and stops only with `--now`); a masked unit is treated as nonexistent,
  but a still-running service is shown as `masked`, and commands such as `start`
  refuse to start it.

- The exit codes of `is-active`, `is-failed` and `is-enabled` match systemd.

- `is-system-running` prints the running state of the whole instance; its exit
  code mimics systemd: `running` returns 0, all other states return 1. During
  early startup it prints `initializing` or `starting`, during shutdown it
  prints `stopping`, when the manager is not running it prints `offline`, and
  when querying the state fails it prints `unknown`. This project currently does
  not use `maintenance`.

- `whoami` sends the request to pm-sysd-init. With a single PID argument it
  checks whether that PID belongs to a running service; on a match it prints
  `Service=NAME`, otherwise it prints an error and returns non-zero. Without an
  argument its meaning differs from systemctl: it returns information about
  pm-sysd-init itself (`InstanceType`, `InstanceName`, `Pid`, plus `ServiceName`
  when running as a Windows service). By default it prints multiple `KEY=VALUE`
  lines; with `--json` it prints a single-line JSON object with the same keys.

- `show` and `cat` print the same set of all supported config fields: `show`
  uses `key=value` (keys match the config, e.g. `Service.ExecStart`), while
  `cat` uses pretty JSON (it does not mimic systemd's unit file format).

- `list-dependencies` returns an empty result because unit dependencies are not
  implemented yet.

- `daemon-reload` re-reads the config file and refreshes the enable / mask
  state; while a reload is in progress, `enable` / `disable` / `reenable` /
  `mask` / `unmask` report an error, and concurrent `daemon-reload` requests are
  not allowed.

- `set-default` initially supports only `default.target`; other values error.
