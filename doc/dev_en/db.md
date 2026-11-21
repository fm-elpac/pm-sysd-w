# Database (db module)

pm-sysd-init uses a SQLite database to store persistent state (unit enable /
mask) and logs. The database format is an internal API, not a stable external
API. File location and configuration are in the user guide.

Log writes are handled cooperatively by the log batch thread and the log writer
thread; see [log.md](log.md) and the `db` module code comments.

## Schema Version and Upgrade

The database must store a schema version number, used for version detection and
automatic upgrade:

- Version check, initialization and upgrade are wrapped in a single transaction;
  a failure rolls back everything.
- Only forward upgrades are allowed; a newer database causes an error exit.
- Before the v0.1.0 release (covering all `0.1.0-*` prereleases), the schema
  version stays at 1. Afterwards, each schema change increments the version and
  adds migration code.

The actual table schema, upgrade flow, thread model and transaction conventions
are in the `db` module's code comments.
