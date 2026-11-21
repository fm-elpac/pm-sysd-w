# Memory Target and Defensive Limits

## Memory Target

Memory target: under reasonable configuration, pm-sysd-init uses at most 100MB
of runtime memory (including all defensive limits). In other words, with only
100MB available it should still run normally without crashing. For this reason
the code introduces various defensive limits.

## Defensive Limits

Defensive limits cover config parsing (config file / unit size), IPC (message
size, timeouts, concurrent connections), config reload (soft / hard timeout),
logging (single-entry length, page size, retention, rate-limited warnings), and
database (channel buffers). All limits are constants with the `SYSD_LIMIT_`
prefix in `src/conf/limit.rs` (the only exception is
`SYSD_CONF_LIMIT_TOTAL_BYTE`); values and behavior are in the code comments.
