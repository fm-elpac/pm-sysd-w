# Logging Module (logd)

`logd` is the core logging logic (journald-like, simplified). Logs are stored in
the SQLite database; database writes are done cooperatively by the `db` module's
log batch thread and log writer thread.

`logd` handles: log writing (including capture of running processes' stdout /
stderr), log queries (paged), and automatic log cleanup. Defensive limits are in
[limit.md](limit.md).

- The public handle is clonable, and writes are non-blocking deliveries.
- The log batch thread keeps an internal buffer and checks both the
  pending-entry count (`SYSD_LIMIT_LOG_PENDING_N`) and the pending total size
  (`SYSD_LIMIT_LOG_PENDING_BYTE`); when either limit is reached it drops the log
  and emits a rate-limited warning. The log writer thread pulls a batch at a
  time and writes it inside a single SQLite transaction.
- The log input MPSC channel capacity (`SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N`)
  limits how many messages can pile up in the channel, so it cannot grow
  unbounded when the batch thread falls behind. Pending log data can use about
  2x the limits in memory: the batch thread's internal buffer, plus the batch
  already handed to the log writer thread.
- Subprocess stdout / stderr are read line-by-line by background tasks with
  streaming decoding (UTF-8 on Linux, ACP fallback on Windows).
- Log queries use paging and keep one page within the single-message length
  limit.

Implementation details are in the `logd` and `db` module code comments.

Stress test and parameter experiment results are in [log_test.md](log_test.md).
