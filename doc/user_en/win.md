# Windows Integration / Exclusive Features

pm-sysd-w targets Windows primarily. Compared with Linux (testing only), Windows
adds:

- `env:Win.CreateNoWindow` config: set `CREATE_NO_WINDOW` when starting a
  process, so console programs show no black window (see
  [config.md](config.md)).

- Windows per-user service auto-start (SCM integration):
  `install/setup.ps1 install-user` installs `pm-sysd-init --user` as a per-user
  service that starts automatically when the user signs in (see
  [install.md](install.md)).

- Read-only connection to a `--system` instance for ordinary users (see
  [ctl.md](ctl.md)).

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
