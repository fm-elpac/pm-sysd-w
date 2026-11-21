# Build / Quick Start / Install

## Build

You currently need to build from source. A Rust toolchain
(<https://rust-lang.org/>) is required; only Windows 10 or newer is supported.

```sh
cargo build --release --features cli_init
cargo build --release --features cli_ctl
cargo build --release --features cli_log
```

The build outputs are under `target/release/`: `pm-sysd-init.exe`,
`pm-sysd-ctl.exe`, `pm-sysd-log.exe`.

## Quick Start

You can skip installation and run `pm-sysd-init` manually for a quick trial; see
the `Install Script` section for installation.

Start an instance (for example the user instance `--user`), specifying the
absolute path of the main config file (see [config.md](config.md)):

```sh
pm-sysd-init --user --conf=path/to/pm_sysd_w.conf.toml
```

Then operate with `pm-sysd-ctl` (see [ctl.md](ctl.md)), for example:

```sh
pm-sysd-ctl --user start test-run

pm-sysd-ctl --user status test-run

pm-sysd-ctl --user stop test-run

pm-sysd-ctl --user enable test-run
```

View logs (see [log.md](log.md)) and exit the instance:

```sh
pm-sysd-log --user -eu test-run.service

pm-sysd-ctl --user exit
```

## Install Script (Experimental)

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

---

`uninstall-user` only deletes the per-user service and leaves the
`%LOCALAPPDATA%\pm-sysd-w\` directory untouched.

> **Experimental support**: only these two commands are supported for now.
> `pm-sysd-init` implements Windows service integration (SCM); the per-user
> service starts automatically when the user signs in. Other auto-start methods
> (Task Scheduler / Startup folder / Run registry key) are still handled
> manually by the user.

## Exiting and Restarting the Instance

- `pm-sysd-ctl exit` exits pm-sysd-init normally.

- `pm-sysd-ctl soft-reboot` restarts pm-sysd-init itself and all managed
  services (not a Windows reboot).

- External signals: `Ctrl+C` on Windows and `SIGTERM` on Linux also trigger a
  graceful exit.
