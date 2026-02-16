use rusqlite::Connection;
use std::path::PathBuf;
use tempfile::TempDir;

/// Create a fresh temporary database, returning the connection, dir handle, and db path.
/// The caller must hold onto `TempDir` to keep the temp directory alive.
pub fn setup_db() -> (Connection, TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = covrs::db::open(&db_path).unwrap();
    covrs::db::init_schema(&conn).unwrap();
    (conn, dir, db_path)
}
