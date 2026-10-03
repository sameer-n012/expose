//! Database schema and migrations.
//!
//! # Rules
//!
//! - `PRAGMA user_version` holds the schema version. Version 0 is an empty
//!   database.
//! - Migrations are forward-only. Never change a migration that was
//!   released. Add a new one at the end of [`MIGRATIONS`].
//! - All pending migrations run in one transaction. If one fails, the
//!   database stays at its old version.
//! - A database with a version higher than [`LATEST_VERSION`] is rejected,
//!   so that old code cannot damage data that it does not know.
//!
//! # Plan for virtual copies
//!
//! Virtual copies (several edit versions of one file) are not in schema
//! version 1. A later migration will add a `versions` table
//! (`id`, `photo_id`, `name`, `created_ms`), make one "master" version row
//! for each photo, and move the key of `develop_settings` from `photo_id`
//! to `version_id`. Until then, each photo has one settings document.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{CatalogError, Result};

/// The application id in the database header. It identifies an Expose
/// catalog file. The value is the ASCII text "EXPS".
pub const APPLICATION_ID: i32 = 0x4558_5053;

/// The migrations. Item `n` changes the schema from version `n` to `n + 1`.
const MIGRATIONS: &[&str] = &[
    // Version 1: folders, photos, develop settings.
    r"
    CREATE TABLE folders (
        id        INTEGER PRIMARY KEY,
        parent_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
        path      TEXT NOT NULL UNIQUE
    ) STRICT;
    CREATE INDEX idx_folders_parent ON folders(parent_id);

    CREATE TABLE photos (
        id               INTEGER PRIMARY KEY,
        folder_id        INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
        file_name        TEXT NOT NULL CHECK (length(file_name) > 0),
        file_size        INTEGER NOT NULL CHECK (file_size >= 0),
        file_modified_ms INTEGER NOT NULL,
        imported_ms      INTEGER NOT NULL,
        capture_time_ms  INTEGER,
        width            INTEGER CHECK (width IS NULL OR width > 0),
        height           INTEGER CHECK (height IS NULL OR height > 0),
        rating           INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
        flag             TEXT NOT NULL DEFAULT 'none'
                         CHECK (flag IN ('none', 'pick', 'reject')),
        color_label      TEXT NOT NULL DEFAULT 'none'
                         CHECK (color_label IN ('none', 'red', 'yellow', 'green', 'blue', 'purple')),
        UNIQUE (folder_id, file_name)
    ) STRICT;
    CREATE INDEX idx_photos_capture  ON photos(folder_id, capture_time_ms);
    CREATE INDEX idx_photos_name     ON photos(folder_id, file_name COLLATE NOCASE);
    CREATE INDEX idx_photos_rating   ON photos(folder_id, rating);
    CREATE INDEX idx_photos_imported ON photos(folder_id, imported_ms);

    CREATE TABLE develop_settings (
        photo_id       INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
        schema_version INTEGER NOT NULL CHECK (schema_version >= 0),
        settings_json  TEXT NOT NULL CHECK (json_valid(settings_json)),
        updated_ms     INTEGER NOT NULL
    ) STRICT;
    ",
];

/// The newest schema version that this code knows.
pub const LATEST_VERSION: u32 = {
    // There are far fewer than u32::MAX migrations, so the cast cannot
    // truncate.
    #[allow(clippy::cast_possible_truncation)]
    let version = MIGRATIONS.len() as u32;
    version
};

/// Reads the schema version of the database.
pub fn schema_version(conn: &Connection) -> Result<u32> {
    let raw: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // SQLite stores user_version as a signed 32-bit value. A negative value
    // was not written by Expose.
    u32::try_from(raw).map_err(|_| CatalogError::CorruptValue {
        field: "user_version",
        value: raw.to_string(),
    })
}

/// Checks that the database is an Expose catalog (or empty), and applies
/// all pending migrations in one transaction.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    // An immediate transaction takes the write lock first, so that two
    // processes cannot migrate the same file at the same time.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version = schema_version(&tx)?;
    if version > LATEST_VERSION {
        return Err(CatalogError::NewerSchema {
            found: version,
            supported: LATEST_VERSION,
        });
    }
    let application_id: i32 = tx.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if version == 0 {
        // An empty database has no tables. A database with tables but no
        // version belongs to a different program: do not change it.
        let table_count: i64 =
            tx.query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))?;
        if table_count > 0 || application_id != 0 {
            return Err(CatalogError::NotACatalog { application_id });
        }
    } else if application_id != APPLICATION_ID {
        return Err(CatalogError::NotACatalog { application_id });
    }
    if version == LATEST_VERSION {
        return Ok(());
    }

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tracing::info!(to_version = index + 1, "apply catalog migration");
        tx.execute_batch(sql)?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.pragma_update(None, "user_version", LATEST_VERSION)?;
    tx.commit()?;
    Ok(())
}
