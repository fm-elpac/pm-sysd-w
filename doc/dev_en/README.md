# pm-sysd-w Developer Documentation

## Reading Guide

This document targets developers and records development / internal content not
covered by the user guide (`doc/user/`).

- Developers must read the user guide first; this document supplements it with
  **design / requirements / overview** and does not repeat content already in
  the user guide.

- Parts that match (or closely resemble) systemd are simplified or omitted here;
  the focus is on this project's **differences** from systemd.

- This document does not repeat code comments or the code (implementation)
  itself. Low-level implementation details (constant values, message types,
  state machine states, table schema, etc.) live in the code comments.

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

## Module Layout

- `cli`: common CLI module and 3 CLI implementations (cli feature only). See
  [cli.md](cli.md).

- `jsonl`: JSONL communication protocol between init and ctl / log. See
  [jsonl.md](jsonl.md).

- `conf`: calls pm-tc to read the TOML config file. Loading is in
  [sysd.md](sysd.md).

- `db`: SQLite database abstraction / wrapper. See [db.md](db.md).

- `logd`: core logging logic (journald-like, simplified). See [log.md](log.md).
  Log stress tests and parameter experiments are in [log_test.md](log_test.md).

- `sysd`: core service-management logic (incl. `sysd::init` resident loop). See
  [sysd.md](sysd.md).

- `sysu`: systemd-style state definitions / print output. See
  [sysu.md](sysu.md).

- `pa` / `p_w` / `p_u`: platform abstraction and Windows / Linux
  implementations. See [win.md](win.md).

---

`unsafe` code notes are in [unsafe.md](unsafe.md).

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
