# CLI Module

The `cli` module (compiled only with the `cli` feature) contains the CLI
implementations of the 3 binaries plus the instance resolution / connection /
handshake logic shared by all 3.

## Binaries and Features

pm-sysd-w produces 3 binaries, each corresponding to one cargo feature:

| Binary         | Role              | Feature    |
| -------------- | ----------------- | ---------- |
| `pm-sysd-init` | Like PID 1        | `cli_init` |
| `pm-sysd-ctl`  | Like `systemctl`  | `cli_ctl`  |
| `pm-sysd-log`  | Like `journalctl` | `cli_log`  |

The default feature set does not include them; each should be enabled and built
separately (commands in `AGENTS.md`).

## Shared Logic

Instance resolution / connection / handshake logic is shared in the `cli`
module; the specific rules (default instance fallback, fast failure, read-only
fallback, etc.) are in the code comments.

## Commands and Output

The command list and options are in the user guide. Output formats and exit
codes are in [sysu.md](sysu.md). Initial-version commands may just print `TODO`
without implementing all functionality.

## CLI Scaffolding

`pm-bin` provides the CLI scaffolding (same as pm-tc).
