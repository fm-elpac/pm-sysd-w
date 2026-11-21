# sysu Module (systemd-Style Output)

`sysu` is the systemd-style state definition / print output module. It only
renders internal state into `systemctl` / `journalctl` style readable output and
exit codes, and does not participate in business logic.

Submodules:

- `sysu::s`: state definitions (kept consistent with systemd as much as
  possible).
- `sysu::ctl`: systemctl-style output and exit codes for pm-sysd-ctl.
- `sysu::log`: journalctl-style output for pm-sysd-log.
- `sysu::p` (internal): systemd-style print / string formatting.

## Style Conventions

- Similar only; not aiming to be fully identical to systemd, and no pager.
- Fields unrelated to this project or impossible to provide are omitted.
- Output does not contain the word `systemd` (this project is not systemd).

The field-level omission list and exact output formats are in the code comments.
