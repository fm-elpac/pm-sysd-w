# Database Documentation

pm-sysd-init uses a SQLite database to store persistent state and logs. The
database format is an internal API and is not a stable external API.

The database file location and the `SYSD_DB` config are in the user guide
(`doc/user/config.md`); the database access method (dedicated threads / MPSC) is
in the developer documentation (`doc/dev/README.md`).

## Version and Upgrades

The database must store a schema version number (the `schema_version` key in the
`pm_sysd_w_meta` table, an integer) for version detection and automatic
upgrades. The opening flow (version check, initialization, and upgrade) is
entirely wrapped in a single transaction:

1. Open the SQLite connection and begin a transaction; all subsequent steps run
   inside that transaction.

2. Check whether the `pm_sysd_w_meta` table exists:

   - Not present: treat as a new database; create tables / indexes / initial
     metadata.
   - Present: read and validate `program` and `schema_version`:

     - `program` is not `pm-sysd-w`: report an error and exit.
     - Current version newer than supported: report an error and exit; no
       downgrade.
     - Current version older than supported: run migrations and update
       `schema_version` and related metadata.

3. Commit the transaction (any failure rolls the whole operation back, so it
   never ends up "half initialized / half upgraded").

For a new database, write the current schema version (`db::SCHEMA_VERSION`),
plus program name / version / timestamp metadata.

Currently `SCHEMA_VERSION = 1`, with no migration steps. Before the v0.1.0
release (covering all `0.1.0-*` pre-releases), the schema version is not bumped.
Each future schema change must bump the version number and add the corresponding
migration code (in the `db` module).

## Tables (v1)

### `pm_sysd_w_meta`

Key-value pairs holding metadata (including the schema version). When opening
the database, check that `program` equals `pm-sysd-w` to avoid confusing this
program's database with another; on mismatch, report an error and exit.

| Column  | Type | Constraint  | Description |
| ------- | ---- | ----------- | ----------- |
| `key`   | TEXT | PRIMARY KEY | Key         |
| `value` | TEXT | NOT NULL    | Value       |

Built-in keys:

| Key                    | Example value              | Description                                                |
| ---------------------- | -------------------------- | ---------------------------------------------------------- |
| `program`              | `pm-sysd-w`                | Program name (checked on open)                             |
| `schema_version`       | `1`                        | Schema version (integer, stored as a string)               |
| `created_by_version`   | `0.1.0-a1`                 | pm-sysd-w version at DB creation (debugging)               |
| `last_upgrade_version` | `0.1.0-a1`                 | pm-sysd-w version that last changed the schema (debugging) |
| `created_at`           | `2026-11-10T00:00:00.000Z` | DB creation time (debugging)                               |
| `updated_at`           | `2026-11-10T00:00:00.000Z` | Schema last update time (debugging)                        |

Times use ISO 8601 (UTC, similar to JS `Date.toISOString()` format).

### `unit`

Persistent state for units (enable / mask, etc.).

| Column    | Type    | Constraint         | Description           |
| --------- | ------- | ------------------ | --------------------- |
| `name`    | TEXT    | PRIMARY KEY        | Unit name             |
| `enabled` | INTEGER | NOT NULL DEFAULT 0 | Whether enabled (0/1) |
| `masked`  | INTEGER | NOT NULL DEFAULT 0 | Whether masked (0/1)  |

The db module provides exactly two unit read/write APIs:

- Read: read all rows of the unit table at once (a list of `UnitState`).
- Write: write the complete state of one unit (`UnitState`, containing `name` /
  `enabled` / `masked`).

Persistent state cleanup: when all written values are the defaults
(`enabled=false` and `masked=false`), the row is deleted to avoid unbounded
growth of this table.

### `log`

Log records (written by the `logd` module).

| Column  | Type    | Constraint                | Description                     |
| ------- | ------- | ------------------------- | ------------------------------- |
| `id`    | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-increment id               |
| `ts`    | TEXT    | NOT NULL                  | Timestamp (UTC, ISO 8601)       |
| `level` | TEXT    | NOT NULL                  | Log level                       |
| `unit`  | TEXT    |                           | Related unit name (may be null) |
| `msg`   | TEXT    | NOT NULL                  | Log message                     |

- Log level: used for filtering, such as `debug` / `info` / `warn` / `error`;
  subprocess output is `stdout` / `stderr`.

### Indexes

| Index           | Table | Column      | Description                                   |
| --------------- | ----- | ----------- | --------------------------------------------- |
| `idx_log_unit`  | `log` | `unit, id`  | Filter logs by unit (queries, ordered by id)  |
| `idx_log_level` | `log` | `level, id` | Filter logs by level (queries, ordered by id) |
| `idx_log_ts`    | `log` | `ts`        | Automatic log cleanup / time-based queries    |
