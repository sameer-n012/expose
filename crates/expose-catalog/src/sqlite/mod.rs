//! The SQLite storage backend.

mod schema;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, Transaction, params, params_from_iter};

pub use schema::{APPLICATION_ID, LATEST_VERSION};

use crate::error::{CatalogError, Result};
use crate::model::{
    AddOutcome, BatchInfoOutcome, ColorLabel, DevelopSettings, EditorOutput, Flag, Folder,
    FolderId, FolderNode, ImageInfo, LabelUpdate, NewPhoto, Photo, PhotoId, Rating,
};
use crate::paths::{
    build_folder_tree, folder_name, is_valid_file_name, normalize_folder_path, path_text,
};
use crate::platform::Clock;
use crate::query::{PhotoQuery, SortDirection, SortField};
use crate::store::CatalogStore;

/// How long a write waits for a lock held by a different connection.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The photo columns, in the order that [`read_photo`] expects.
const PHOTO_COLUMNS: &str = "p.id, p.folder_id, p.file_name, p.file_size, p.file_modified_ms, \
     p.imported_ms, p.capture_time_ms, p.width, p.height, p.rating, p.flag, p.color_label, \
     EXISTS (SELECT 1 FROM editor_outputs e WHERE e.photo_id = p.id)";

/// A catalog stored in one SQLite database file.
///
/// The store uses WAL journal mode and enforces foreign keys. One store
/// holds one connection. It is `Send` but not `Sync`: to share it between
/// threads, put it in a mutex.
pub struct SqliteStore {
    conn: Connection,
    clock: Box<dyn Clock + Send>,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore")
            .field("path", &self.conn.path())
            .finish_non_exhaustive()
    }
}

impl SqliteStore {
    /// Opens the catalog file at `path`. Makes a new catalog if the file
    /// does not exist. Applies pending schema migrations.
    ///
    /// The store uses the system clock.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::NewerSchema`] if a newer version of Expose
    /// made the file, and [`CatalogError::NotACatalog`] if the file is a
    /// different database.
    #[cfg(feature = "native")]
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_clock(path, Box::new(crate::platform::SystemClock))
    }

    /// Opens the catalog file at `path` with a given clock. See
    /// [`SqliteStore::open`].
    pub fn open_with_clock(path: &Path, clock: Box<dyn Clock + Send>) -> Result<Self> {
        Self::init(Connection::open(path)?, clock)
    }

    /// Makes a new, empty catalog in memory. Use it for tests.
    #[cfg(feature = "native")]
    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with_clock(Box::new(crate::platform::SystemClock))
    }

    /// Makes a new, empty catalog in memory with a given clock.
    pub fn open_in_memory_with_clock(clock: Box<dyn Clock + Send>) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, clock)
    }

    fn init(mut conn: Connection, clock: Box<dyn Clock + Send>) -> Result<Self> {
        // These settings apply to the connection, not to the file. They
        // cannot change inside a transaction, so set them first.
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        schema::migrate(&mut conn)?;
        // WAL lets readers continue while a write occurs. An in-memory
        // database reports "memory"; that is correct for it.
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "wal", |row| row.get(0))?;
        tracing::debug!(journal_mode = %mode, "catalog opened");
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(Self { conn, clock })
    }

    /// Returns the schema version of the open catalog.
    pub fn schema_version(&self) -> Result<u32> {
        schema::schema_version(&self.conn)
    }
}

// --- Conversions between domain types and SQL values ---

fn conversion_error(error: impl std::error::Error + Send + Sync + 'static) -> FromSqlError {
    FromSqlError::Other(Box::new(error))
}

impl ToSql for Rating {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(i64::from(self.stars())))
    }
}

impl FromSql for Rating {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let stars = u8::column_result(value)?;
        Self::new(stars).map_err(conversion_error)
    }
}

/// The error for stored text that is not a known enum value.
#[derive(Debug, thiserror::Error)]
#[error("unknown stored value {0:?}")]
struct UnknownValue(String);

impl ToSql for Flag {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for Flag {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::from_str_opt(text).ok_or_else(|| conversion_error(UnknownValue(text.to_owned())))
    }
}

impl ToSql for ColorLabel {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for ColorLabel {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Self::from_str_opt(text).ok_or_else(|| conversion_error(UnknownValue(text.to_owned())))
    }
}

fn read_photo(row: &Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: PhotoId(row.get(0)?),
        folder_id: FolderId(row.get(1)?),
        file_name: row.get(2)?,
        file_size: row.get(3)?,
        file_modified_ms: row.get(4)?,
        imported_ms: row.get(5)?,
        capture_time_ms: row.get(6)?,
        width: row.get(7)?,
        height: row.get(8)?,
        rating: row.get(9)?,
        flag: row.get(10)?,
        color_label: row.get(11)?,
        is_editor_output: row.get(12)?,
    })
}

fn read_editor_output(row: &Row<'_>) -> rusqlite::Result<EditorOutput> {
    Ok(EditorOutput {
        photo_id: PhotoId(row.get(0)?),
        document_path: PathBuf::from(row.get::<_, String>(1)?),
        source_photo_id: row.get::<_, Option<i64>>(2)?.map(PhotoId),
    })
}

/// Splits a normalized file path into its folder path and file name.
fn split_file_path(path: &Path) -> Result<(&Path, &str)> {
    let invalid = |reason| CatalogError::InvalidPath {
        path: path.to_path_buf(),
        reason,
    };
    let folder = path
        .parent()
        .ok_or_else(|| invalid("the path has no folder"))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| is_valid_file_name(n))
        .ok_or_else(|| invalid("the path has no valid file name"))?;
    Ok((folder, name))
}

fn photo_id_by_path(conn: &Connection, path: &Path) -> Result<Option<PhotoId>> {
    let (folder, name) = split_file_path(path)?;
    let id = conn
        .prepare_cached(
            "SELECT p.id FROM photos p JOIN folders f ON f.id = p.folder_id \
             WHERE f.path = ?1 AND p.file_name = ?2",
        )?
        .query_row(params![path_text(folder)?, name], |row| row.get(0))
        .optional()?;
    Ok(id.map(PhotoId))
}

fn read_folder(row: &Row<'_>) -> rusqlite::Result<Folder> {
    let path = PathBuf::from(row.get::<_, String>(2)?);
    Ok(Folder {
        id: FolderId(row.get(0)?),
        parent_id: row.get::<_, Option<i64>>(1)?.map(FolderId),
        name: folder_name(&path),
        path,
    })
}

// --- Folder helpers (run inside a transaction) ---

fn folder_id_by_path(conn: &Connection, path: &Path) -> Result<Option<FolderId>> {
    let id = conn
        .prepare_cached("SELECT id FROM folders WHERE path = ?1")?
        .query_row([path_text(path)?], |row| row.get(0))
        .optional()?;
    Ok(id.map(FolderId))
}

fn load_folder(conn: &Connection, id: FolderId) -> Result<Option<Folder>> {
    Ok(conn
        .prepare_cached("SELECT id, parent_id, path FROM folders WHERE id = ?1")?
        .query_row([id.0], read_folder)
        .optional()?)
}

fn insert_folder(tx: &Transaction<'_>, parent: Option<FolderId>, path: &Path) -> Result<FolderId> {
    tx.prepare_cached("INSERT INTO folders (parent_id, path) VALUES (?1, ?2)")?
        .execute(params![parent.map(|p| p.0), path_text(path)?])?;
    Ok(FolderId(tx.last_insert_rowid()))
}

/// Returns the id of the folder at `path` (normalized). Adds the folder if
/// it is not in the catalog, and keeps the tree free of gaps:
///
/// 1. Find the nearest ancestor folder that the catalog has (the anchor).
/// 2. Add each folder from below the anchor down to `path`.
/// 3. Move each folder that is below `path` but has the anchor as its
///    parent (or no parent, if there is no anchor) under `path`. This step
///    also adds the folders between `path` and the moved folder.
fn ensure_folder(tx: &Transaction<'_>, path: &Path) -> Result<FolderId> {
    if let Some(id) = folder_id_by_path(tx, path)? {
        return Ok(id);
    }

    let mut anchor: Option<(FolderId, &Path)> = None;
    for ancestor in path.ancestors().skip(1) {
        if let Some(id) = folder_id_by_path(tx, ancestor)? {
            anchor = Some((id, ancestor));
            break;
        }
    }

    // Folders between the anchor (exclusive) and `path` (exclusive),
    // nearest to the anchor first. With no anchor, `path` becomes a root.
    let mut between: Vec<&Path> = match anchor {
        Some((_, anchor_path)) => path
            .ancestors()
            .skip(1)
            .take_while(|p| *p != anchor_path)
            .collect(),
        None => Vec::new(),
    };
    between.reverse();

    let anchor_id = anchor.map(|(id, _)| id);
    let mut parent = anchor_id;
    for intermediate in between {
        parent = Some(insert_folder(tx, parent, intermediate)?);
    }
    let id = insert_folder(tx, parent, path)?;

    // Folders below `path` that were attached to the anchor (or were roots).
    let mut adopt = Vec::new();
    {
        let mut stmt = tx.prepare_cached("SELECT id, path FROM folders WHERE parent_id IS ?1")?;
        let rows = stmt.query_map([anchor_id.map(|a| a.0)], |row| {
            Ok((
                FolderId(row.get(0)?),
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })?;
        for row in rows {
            let (child_id, child_path) = row?;
            if child_path != path && child_path.starts_with(path) {
                adopt.push((child_id, child_path));
            }
        }
    }
    for (child_id, child_path) in adopt {
        // `child_path` is strictly below `path`, so it has a parent path.
        let Some(child_parent) = child_path.parent() else {
            continue;
        };
        let new_parent = ensure_folder(tx, child_parent)?;
        tx.prepare_cached("UPDATE folders SET parent_id = ?1 WHERE id = ?2")?
            .execute([new_parent.0, child_id.0])?;
    }
    Ok(id)
}

fn photo_exists(conn: &Connection, id: PhotoId) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM photos WHERE id = ?1")?
        .exists([id.0])?)
}

/// Builds the `FROM ... WHERE ...` part of a photo query and its
/// parameters. Parameter `?1` is the folder id.
fn photo_query_body(query: &PhotoQuery) -> (String, Vec<Value>) {
    let mut sql = if query.recursive {
        String::from(
            "WITH RECURSIVE scope(id) AS ( \
                 SELECT ?1 UNION ALL \
                 SELECT f.id FROM folders f JOIN scope s ON f.parent_id = s.id) \
             SELECT {cols} FROM photos p WHERE p.folder_id IN (SELECT id FROM scope)",
        )
    } else {
        String::from("SELECT {cols} FROM photos p WHERE p.folder_id = ?1")
    };
    let mut values = vec![Value::Integer(query.folder_id.0)];
    let filter = &query.filter;
    if let Some(min) = filter.min_rating {
        push_param(
            &mut sql,
            &mut values,
            " AND p.rating >= ",
            i64::from(min.stars()),
        );
    }
    if let Some(flag) = filter.flag {
        push_param(
            &mut sql,
            &mut values,
            " AND p.flag = ",
            flag.as_str().to_owned(),
        );
    }
    if let Some(label) = filter.color_label {
        push_param(
            &mut sql,
            &mut values,
            " AND p.color_label = ",
            label.as_str().to_owned(),
        );
    }
    (sql, values)
}

/// Appends `prefix` and a numbered parameter (`?n`) to `sql`, and appends
/// the parameter value to `values`.
fn push_param(sql: &mut String, values: &mut Vec<Value>, prefix: &str, value: impl Into<Value>) {
    values.push(value.into());
    sql.push_str(prefix);
    sql.push('?');
    sql.push_str(&values.len().to_string());
}

fn order_by_clause(query: &PhotoQuery) -> String {
    let dir = match query.sort.direction {
        SortDirection::Ascending => "ASC",
        SortDirection::Descending => "DESC",
    };
    let primary = match query.sort.field {
        // Photos with no capture time are last in both directions.
        SortField::CaptureTime => format!("p.capture_time_ms IS NULL, p.capture_time_ms {dir}"),
        SortField::FileName => format!("p.file_name COLLATE NOCASE {dir}"),
        SortField::Rating => format!("p.rating {dir}"),
        SortField::ImportTime => format!("p.imported_ms {dir}"),
    };
    // Tie-breakers make the order deterministic.
    format!(" ORDER BY {primary}, p.file_name ASC, p.id ASC")
}

impl CatalogStore for SqliteStore {
    fn add_folder(&mut self, path: &Path) -> Result<Folder> {
        let path = normalize_folder_path(path)?;
        let tx = self.conn.transaction()?;
        let id = ensure_folder(&tx, &path)?;
        let folder = load_folder(&tx, id)?.ok_or(CatalogError::FolderNotFound(id))?;
        tx.commit()?;
        Ok(folder)
    }

    fn get_folder(&self, id: FolderId) -> Result<Option<Folder>> {
        load_folder(&self.conn, id)
    }

    fn find_folder(&self, path: &Path) -> Result<Option<Folder>> {
        let path = normalize_folder_path(path)?;
        match folder_id_by_path(&self.conn, &path)? {
            Some(id) => load_folder(&self.conn, id),
            None => Ok(None),
        }
    }

    fn list_folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, parent_id, path FROM folders ORDER BY path")?;
        let folders = stmt
            .query_map([], read_folder)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(folders)
    }

    fn list_folder_tree(&self) -> Result<Vec<FolderNode>> {
        let folders = self.list_folders()?;
        let mut stmt = self
            .conn
            .prepare_cached("SELECT folder_id, count(*) FROM photos GROUP BY folder_id")?;
        let counts = stmt
            .query_map([], |row| Ok((FolderId(row.get(0)?), row.get::<_, u64>(1)?)))?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(build_folder_tree(folders, &counts))
    }

    fn add_photos(&mut self, folder_id: FolderId, photos: &[NewPhoto]) -> Result<Vec<AddOutcome>> {
        if let Some(bad) = photos.iter().find(|p| !is_valid_file_name(&p.file_name)) {
            return Err(CatalogError::InvalidPath {
                path: PathBuf::from(&bad.file_name),
                reason: "the file name is empty or contains a path separator",
            });
        }
        let now = self.clock.now_ms();
        let tx = self.conn.transaction()?;
        if load_folder(&tx, folder_id)?.is_none() {
            return Err(CatalogError::FolderNotFound(folder_id));
        }
        let mut outcomes = Vec::with_capacity(photos.len());
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO photos (folder_id, file_name, file_size, file_modified_ms, imported_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT (folder_id, file_name) DO NOTHING",
            )?;
            let mut existing =
                tx.prepare_cached("SELECT id FROM photos WHERE folder_id = ?1 AND file_name = ?2")?;
            for photo in photos {
                let changed = insert.execute(params![
                    folder_id.0,
                    photo.file_name,
                    photo.file_size,
                    photo.file_modified_ms,
                    now
                ])?;
                outcomes.push(if changed == 1 {
                    AddOutcome::Added(PhotoId(tx.last_insert_rowid()))
                } else {
                    let id = existing
                        .query_row(params![folder_id.0, photo.file_name], |row| row.get(0))?;
                    AddOutcome::AlreadyKnown(PhotoId(id))
                });
            }
        }
        tx.commit()?;
        Ok(outcomes)
    }

    fn known_file_names(&self, folder_id: FolderId) -> Result<HashSet<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT file_name FROM photos WHERE folder_id = ?1")?;
        let names = stmt
            .query_map([folder_id.0], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?;
        Ok(names)
    }

    fn get_photo(&self, id: PhotoId) -> Result<Option<Photo>> {
        let sql = format!("SELECT {PHOTO_COLUMNS} FROM photos p WHERE p.id = ?1");
        Ok(self
            .conn
            .prepare_cached(&sql)?
            .query_row([id.0], read_photo)
            .optional()?)
    }

    fn photo_path(&self, id: PhotoId) -> Result<Option<PathBuf>> {
        let row = self
            .conn
            .prepare_cached(
                "SELECT f.path, p.file_name FROM photos p \
                 JOIN folders f ON f.id = p.folder_id WHERE p.id = ?1",
            )?
            .query_row([id.0], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;
        Ok(row.map(|(folder, file)| Path::new(&folder).join(file)))
    }

    fn list_photos(&self, query: &PhotoQuery) -> Result<Vec<Photo>> {
        if load_folder(&self.conn, query.folder_id)?.is_none() {
            return Err(CatalogError::FolderNotFound(query.folder_id));
        }
        let (body, mut values) = photo_query_body(query);
        let mut sql = body.replace("{cols}", PHOTO_COLUMNS);
        sql.push_str(&order_by_clause(query));
        // A negative limit means "no limit" in SQLite.
        push_param(
            &mut sql,
            &mut values,
            " LIMIT ",
            query.limit.map_or(-1, i64::from),
        );
        push_param(&mut sql, &mut values, " OFFSET ", i64::from(query.offset));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let photos = stmt
            .query_map(params_from_iter(values), read_photo)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(photos)
    }

    fn count_photos(&self, query: &PhotoQuery) -> Result<u64> {
        if load_folder(&self.conn, query.folder_id)?.is_none() {
            return Err(CatalogError::FolderNotFound(query.folder_id));
        }
        let (body, values) = photo_query_body(query);
        let sql = body.replace("{cols}", "count(*)");
        let count = self
            .conn
            .prepare_cached(&sql)?
            .query_row(params_from_iter(values), |row| row.get(0))?;
        Ok(count)
    }

    fn set_image_info(&mut self, id: PhotoId, info: ImageInfo) -> Result<()> {
        if info.width == Some(0) {
            return Err(CatalogError::ZeroDimension { field: "width" });
        }
        if info.height == Some(0) {
            return Err(CatalogError::ZeroDimension { field: "height" });
        }
        let changed = self
            .conn
            .prepare_cached(
                "UPDATE photos SET capture_time_ms = ?1, width = ?2, height = ?3 WHERE id = ?4",
            )?
            .execute(params![info.capture_time_ms, info.width, info.height, id.0])?;
        if changed == 0 {
            return Err(CatalogError::PhotoNotFound(id));
        }
        Ok(())
    }

    fn set_image_infos(&mut self, items: &[(PhotoId, ImageInfo)]) -> Result<BatchInfoOutcome> {
        let mut outcome = BatchInfoOutcome::default();
        if items.is_empty() {
            return Ok(outcome);
        }
        // One transaction holds the whole batch (SQLite handles it without
        // chunks). Any early return drops `tx`, which rolls back.
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE photos SET capture_time_ms = ?1, width = ?2, height = ?3 WHERE id = ?4",
            )?;
            for &(id, info) in items {
                if info.width == Some(0) {
                    return Err(CatalogError::ZeroDimension { field: "width" });
                }
                if info.height == Some(0) {
                    return Err(CatalogError::ZeroDimension { field: "height" });
                }
                let changed =
                    stmt.execute(params![info.capture_time_ms, info.width, info.height, id.0])?;
                if changed == 0 {
                    outcome.unknown.push(id);
                } else {
                    outcome.written += 1;
                }
            }
        }
        tx.commit()?;
        Ok(outcome)
    }

    fn update_labels(&mut self, ids: &[PhotoId], update: LabelUpdate) -> Result<()> {
        if ids.is_empty() || update.is_empty() {
            return Ok(());
        }
        // `COALESCE(?n, column)` keeps the column when the value is NULL,
        // so one statement serves all combinations of fields.
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE photos SET \
                     rating = COALESCE(?1, rating), \
                     flag = COALESCE(?2, flag), \
                     color_label = COALESCE(?3, color_label) \
                 WHERE id = ?4",
            )?;
            for &id in ids {
                let changed = stmt.execute(params![
                    update.rating,
                    update.flag,
                    update.color_label,
                    id.0
                ])?;
                if changed == 0 {
                    // Dropping the transaction rolls back all earlier rows.
                    return Err(CatalogError::PhotoNotFound(id));
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn get_develop_settings(&self, id: PhotoId) -> Result<Option<DevelopSettings>> {
        if !photo_exists(&self.conn, id)? {
            return Err(CatalogError::PhotoNotFound(id));
        }
        let row = self
            .conn
            .prepare_cached(
                "SELECT schema_version, settings_json, updated_ms \
                 FROM develop_settings WHERE photo_id = ?1",
            )?
            .query_row([id.0], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .optional()?;
        let Some((schema_version, json, updated_ms)) = row else {
            return Ok(None);
        };
        Ok(Some(DevelopSettings {
            photo_id: id,
            schema_version,
            settings: serde_json::from_str(&json)?,
            updated_ms,
        }))
    }

    fn set_develop_settings(
        &mut self,
        id: PhotoId,
        schema_version: u32,
        settings: &serde_json::Value,
    ) -> Result<DevelopSettings> {
        let json = serde_json::to_string(settings)?;
        let now = self.clock.now_ms();
        let tx = self.conn.transaction()?;
        if !photo_exists(&tx, id)? {
            return Err(CatalogError::PhotoNotFound(id));
        }
        tx.prepare_cached(
            "INSERT INTO develop_settings (photo_id, schema_version, settings_json, updated_ms) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (photo_id) DO UPDATE SET \
                 schema_version = excluded.schema_version, \
                 settings_json = excluded.settings_json, \
                 updated_ms = excluded.updated_ms",
        )?
        .execute(params![id.0, schema_version, json, now])?;
        tx.commit()?;
        Ok(DevelopSettings {
            photo_id: id,
            schema_version,
            settings: settings.clone(),
            updated_ms: now,
        })
    }

    fn set_editor_output(&mut self, output: &EditorOutput) -> Result<()> {
        let document_path = normalize_folder_path(&output.document_path)?;
        let document_text = path_text(&document_path)?;
        let now = self.clock.now_ms();
        let tx = self.conn.transaction()?;
        if !photo_exists(&tx, output.photo_id)? {
            return Err(CatalogError::PhotoNotFound(output.photo_id));
        }
        if let Some(source) = output.source_photo_id {
            if !photo_exists(&tx, source)? {
                return Err(CatalogError::PhotoNotFound(source));
            }
        }
        tx.prepare_cached(
            "INSERT INTO editor_outputs \
                 (photo_id, document_path, source_photo_id, created_ms, updated_ms) \
             VALUES (?1, ?2, ?3, ?4, ?4) \
             ON CONFLICT (photo_id) DO UPDATE SET \
                 document_path = excluded.document_path, \
                 source_photo_id = excluded.source_photo_id, \
                 updated_ms = excluded.updated_ms",
        )?
        .execute(params![
            output.photo_id.0,
            document_text,
            output.source_photo_id.map(|p| p.0),
            now
        ])?;
        tx.commit()?;
        Ok(())
    }

    fn editor_output(&self, photo_id: PhotoId) -> Result<Option<EditorOutput>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT photo_id, document_path, source_photo_id \
                 FROM editor_outputs WHERE photo_id = ?1",
            )?
            .query_row([photo_id.0], read_editor_output)
            .optional()?)
    }

    fn editor_output_for_document(&self, document_path: &Path) -> Result<Option<EditorOutput>> {
        let document_path = normalize_folder_path(document_path)?;
        Ok(self
            .conn
            .prepare_cached(
                "SELECT photo_id, document_path, source_photo_id \
                 FROM editor_outputs WHERE document_path = ?1",
            )?
            .query_row([path_text(&document_path)?], read_editor_output)
            .optional()?)
    }

    fn find_photo_by_path(&self, path: &Path) -> Result<Option<PhotoId>> {
        let path = normalize_folder_path(path)?;
        photo_id_by_path(&self.conn, &path)
    }

    fn is_editor_output_path(&self, path: &Path) -> Result<bool> {
        let path = normalize_folder_path(path)?;
        if self
            .conn
            .prepare_cached("SELECT 1 FROM editor_outputs WHERE document_path = ?1")?
            .exists([path_text(&path)?])?
        {
            return Ok(true);
        }
        match photo_id_by_path(&self.conn, &path)? {
            Some(id) => Ok(self
                .conn
                .prepare_cached("SELECT 1 FROM editor_outputs WHERE photo_id = ?1")?
                .exists([id.0])?),
            None => Ok(false),
        }
    }
}
