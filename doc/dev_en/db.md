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
upgrades. Rules:

- New database: write the current schema version (`db::SCHEMA_VERSION`), plus
  program name / version / timestamp metadata.
- Older database: run the corresponding migration steps in order, upgrading step
  by step to the current version.
- Newer database (created by a newer program version): report an error and exit;
  no downgrade.
- After migration, update `schema_version` and related metadata.

Currently `SCHEMA_VERSION = 1` with no migration steps. Each future schema
change must: bump the version number and add the corresponding migration code
(in the `db` module).

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

## Access

All database access goes through the `db` module wrapper; other modules do not
touch SQLite directly. See `src/db/`.
