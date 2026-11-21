# pm-sysd-log Commands and Log Notes

`pm-sysd-log` views logs (similar to `journalctl`). When no instance is
specified, it tries `--system` first; if the system instance is not running, it
falls back to `--user`. When connecting to `--system`, an ordinary
(non-administrator) user automatically uses a read-only connection.

## Usage

```sh
# show recent logs (jump to the end, default)
pm-sysd-log --user

# show recent logs for the specified unit only (-eu combined)
pm-sysd-log --user -eu test-run
```

## Common Options

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

- `-u` / `--unit` unit names without a suffix get `.service` appended
  automatically.

## Log Notes

- Logs are kept for the last 72 hours only; pm-sysd-init runs an automatic
  cleanup once per hour.

- A service process's stdout / stderr is written line-by-line to that unit's
  logs. Over-long lines are split into multiple log entries; the output is not
  guaranteed to be UTF-8, and a decode failure replaces it with one error log
  entry.
