# sysd Module (Service Management Core)

`sysd` is the core service-management logic. The initial version supports only
the `service` unit type.

## Multi-Actor Architecture

The `sysd` module of pm-sysd-init uses a multi-actor architecture: each key
component is a `tokio::spawn`ed async task holding internal state, receiving and
processing messages single-threaded, with MPSC channels between components.

Component handles are clonable. Message sends are non-blocking (wait only for
delivery, not for processing / reply).

Naming convention: `M` suffix = message, `SM` suffix = state machine. Each
component consists of an actor (does IO) and a state machine (pure-computation
decision).

## Components

| Component         | Responsibility                     |
| ----------------- | ---------------------------------- |
| `sysd::init`      | init main loop (resident logic)    |
| `sysd::sys_d`     | top-level entity, routing only     |
| `sysd::unit_d`    | manages all units, starts ServiceD |
| `sysd::service_d` | manages all services               |
| `sysd::service_s` | manages a single service           |
| `sysd::timer_d`   | manages all timers (reserved)      |
| `sysd::timer_s`   | a single timer (reserved)          |

Component responsibilities and message / state machine details are in the code
comments.

## Instance Uniqueness

The same instance (same IPC endpoint name) can have only one process; a
duplicate start exits with an error. Uniqueness detection is creating and
holding the IPC endpoint, done before all other operations, to avoid TOCTOU.

## Configuration Loading

Uses pm-tc's centralized configuration (`[[t.sysd]]`). Configuration loading and
mask / enable state reading are unified in `sysd::reload`, shared between
startup loading and daemon-reload. The config source is abstracted by the
`Reloader` trait: the production `SelfReloader` loads config via the
`pm-sysd-init --reload` subprocess, while tests can inject a custom Reloader to
skip that subprocess.

## Event-Driven

The service lifecycle is event-driven: subprocess exit / timer expiry are
reported by background tasks, so a single unit operation does not block the main
loop. A running service snapshots its configuration at startup and keeps the old
configuration after daemon-reload.

## Graceful Exit and soft-reboot

Exit / soft-reboot is decided by SysD: first stop all services, then notify
init. After exit, init records logs, waits for them to flush, and runs one
vacuum on the database. soft-reboot restarts itself with the same command-line
arguments.

The startup flow (argument parsing, uniqueness detection, config loading,
database opening, starting default.target, entering the IPC main loop) is in the
`sysd::init` code comments.
