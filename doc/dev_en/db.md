# Database Schema

pm-sysd-init uses a SQLite database to store persistent state and logs. The
database format is an internal API and is not a stable external API.

## File Location

The default database file is named `pm_sysd_w_v1.db`, in the same directory as
the main config (if the main config is `dir/XXX.toml`, the database file is
`dir/pm_sysd_w_v1.db`). The path can be changed via `[env] SYSD_DB` in the main
config file (default `pm_sysd_w_v1.db`; a relative path is resolved relative to
the main config's directory).

## Version and Upgrades

The database must store a schema version number (the `schema_version` key in the
`pm_sysd_w_meta` table, an integer) for version detection and automatic
upgrades. Opening flow:

1. First try to read the schema version and the `program` identifier; do not
   write to the database before the version is confirmed.
2. Only when the `pm_sysd_w_meta` table does not exist, create the database
   schema (i.e. a new database).
3. Only when the current version is older than the version supported by this
   program (an upgrade is needed), run migrations; migrations are wrapped in a
   single transaction and complete atomically (either fully succeed or fully
   fail, never "half upgraded").
4. Opening a newer database (created by a newer program): report an error and
   exit; no downgrade.
5. After migration, update `schema_version` and related metadata.

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

### `log`

Log records (written by the `logd` module).

| Column  | Type    | Constraint                | Description                                                                                               |
| ------- | ------- | ------------------------- | --------------------------------------------------------------------------------------------------------- |
| `id`    | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-increment id                                                                                         |
| `ts`    | TEXT    | NOT NULL                  | Timestamp (UTC, ISO 8601)                                                                                 |
| `level` | TEXT    | NOT NULL                  | Log level for filtering (e.g. `debug`/`info`/`warn`/`error`; child process output uses `stdout`/`stderr`) |
| `unit`  | TEXT    |                           | Related unit name (may be null)                                                                           |
| `msg`   | TEXT    | NOT NULL                  | Log message                                                                                               |

### Indexes

| Index          | Table | Column | Description                                |
| -------------- | ----- | ------ | ------------------------------------------ |
| `idx_log_unit` | `log` | `unit` | Filter logs by unit (queries)              |
| `idx_log_ts`   | `log` | `ts`   | Automatic log cleanup / time-based queries |

## Access

All database access goes through the `db` module wrapper; other modules do not
touch SQLite directly. The `db` module starts 2 dedicated (synchronous) threads:
a database read/write thread and a log writer thread; only those two threads
touch SQLite directly. Tokio async threads send requests to the database
read/write thread over MPSC; each request carries a response MPSC, and the
read/write thread sends the result back after finishing. See `src/db/`.
