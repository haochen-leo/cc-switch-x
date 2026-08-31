use super::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, OpenFlags, Transaction};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const SUPPORTED_SOURCE_SCHEMA_VERSION: i32 = super::SCHEMA_VERSION;
const IMPORT_TABLES: &[&str] = &[
    "providers",
    "provider_endpoints",
    "mcp_servers",
    "prompts",
    "skills",
    "skill_repos",
    "profiles",
];
const SKIPPED_SETTING_KEYS: &[&str] = &[
    "skills_ssot_migration_pending",
    "proxy_enabled",
    "live_takeover_active",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportReport {
    pub source_path: String,
    pub source_schema_version: i32,
    pub imported_rows: usize,
    pub imported_settings_file: bool,
    pub imported_skill_files: usize,
}

impl Database {
    pub fn import_from_official_data_dir(
        &self,
        source_dir: &Path,
    ) -> Result<LegacyImportReport, AppError> {
        let source_db_path = source_dir.join("cc-switch.db");
        if !source_db_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "未找到可导入的数据库：{}",
                source_db_path.display()
            )));
        }

        let source = Connection::open_with_flags(
            &source_db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| AppError::Database(format!("只读打开来源数据库失败: {e}")))?;
        let source_schema_version = Self::get_user_version(&source)?;
        if source_schema_version > SUPPORTED_SOURCE_SCHEMA_VERSION {
            return Err(AppError::Database(format!(
                "来源数据库版本为 v{source_schema_version}，当前导入器最高支持 v{SUPPORTED_SOURCE_SCHEMA_VERSION}"
            )));
        }

        let imported_rows = {
            let mut destination = lock_conn!(self.conn);
            let tx = destination
                .transaction()
                .map_err(|e| AppError::Database(format!("开启导入事务失败: {e}")))?;
            let mut count = 0usize;

            for table in IMPORT_TABLES {
                count += copy_common_table(&source, &tx, table)?;
            }
            count += copy_settings(&source, &tx)?;
            import_legacy_retry_config(&source, &tx)?;

            tx.execute(
                "INSERT INTO x_import_history
                 (source_path, source_schema_version, imported_rows)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    source_db_path.to_string_lossy(),
                    source_schema_version,
                    count as i64
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            tx.commit()
                .map_err(|e| AppError::Database(format!("提交导入事务失败: {e}")))?;
            count
        };

        let destination_dir = crate::config::get_app_config_dir();
        let imported_settings_file = import_settings_file(source_dir, &destination_dir)?;
        let imported_skill_files = copy_directory_without_symlinks(
            &source_dir.join("skills"),
            &destination_dir.join("skills"),
        )?;

        Ok(LegacyImportReport {
            source_path: source_db_path.to_string_lossy().to_string(),
            source_schema_version,
            imported_rows,
            imported_settings_file,
            imported_skill_files,
        })
    }
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, AppError> {
    if !Database::table_exists(conn, table)? {
        return Ok(Vec::new());
    }
    let sql = format!("PRAGMA table_info(\"{table}\")");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(columns)
}

fn copy_common_table(
    source: &Connection,
    destination: &Transaction<'_>,
    table: &str,
) -> Result<usize, AppError> {
    let source_columns = table_columns(source, table)?;
    if source_columns.is_empty() {
        return Ok(0);
    }
    let destination_columns: HashSet<String> =
        table_columns(destination, table)?.into_iter().collect();
    let columns: Vec<String> = source_columns
        .into_iter()
        .filter(|column| destination_columns.contains(column))
        .collect();
    if columns.is_empty() {
        return Ok(0);
    }

    let quoted_columns = columns
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!("SELECT {quoted_columns} FROM \"{table}\"");
    let insert_sql =
        format!("INSERT OR REPLACE INTO \"{table}\" ({quoted_columns}) VALUES ({placeholders})");

    let mut source_stmt = source
        .prepare(&select_sql)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut rows = source_stmt
        .query([])
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut imported = 0usize;
    while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
        let values = (0..columns.len())
            .map(|index| row.get::<_, Value>(index))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        destination
            .execute(&insert_sql, params_from_iter(values.iter()))
            .map_err(|e| AppError::Database(e.to_string()))?;
        imported += 1;
    }
    Ok(imported)
}

fn copy_settings(source: &Connection, destination: &Transaction<'_>) -> Result<usize, AppError> {
    if !Database::table_exists(source, "settings")? {
        return Ok(0);
    }
    let mut stmt = source
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut imported = 0usize;
    while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
        let key: String = row.get(0).map_err(|e| AppError::Database(e.to_string()))?;
        if SKIPPED_SETTING_KEYS.contains(&key.as_str()) {
            continue;
        }
        let value: Option<String> = row.get(1).map_err(|e| AppError::Database(e.to_string()))?;
        destination
            .execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        imported += 1;
    }
    Ok(imported)
}

fn import_legacy_retry_config(
    source: &Connection,
    destination: &Transaction<'_>,
) -> Result<(), AppError> {
    if !Database::table_exists(source, "proxy_config")?
        || !Database::has_column(source, "proxy_config", "retry_429_enabled")?
    {
        return Ok(());
    }

    let mut stmt = source
        .prepare(
            "SELECT app_type, retry_429_enabled, retry_429_max_retries,
                    retry_429_initial_delay_ms
             FROM proxy_config",
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| AppError::Database(e.to_string()))?;
    for row in rows {
        let (app_type, enabled, max_retries, initial_delay_ms) =
            row.map_err(|e| AppError::Database(e.to_string()))?;
        destination
            .execute(
                "INSERT OR REPLACE INTO x_proxy_retry_config
                 (app_type, enabled, max_retries, initial_delay_ms, updated_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![app_type, enabled, max_retries, initial_delay_ms],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
    }
    Ok(())
}

fn import_settings_file(source_dir: &Path, destination_dir: &Path) -> Result<bool, AppError> {
    let source_path = source_dir.join("settings.json");
    if !source_path.exists() {
        return Ok(false);
    }
    let content = fs::read_to_string(&source_path).map_err(|e| AppError::io(&source_path, e))?;
    let mut value: JsonValue =
        serde_json::from_str(&content).map_err(|e| AppError::json(&source_path, e))?;
    if let Some(object) = value.as_object_mut() {
        for key in [
            "launchOnStartup",
            "silentStartup",
            "enableLocalProxy",
            "proxyConfirmed",
            "webdavSync",
            "s3Sync",
            "localMigrations",
        ] {
            object.remove(key);
        }
    }
    fs::create_dir_all(destination_dir).map_err(|e| AppError::io(destination_dir, e))?;
    let output = serde_json::to_vec_pretty(&value)
        .map_err(|e| AppError::Config(format!("序列化导入设置失败: {e}")))?;
    crate::config::atomic_write(&destination_dir.join("settings.json"), &output)?;
    Ok(true)
}

fn copy_directory_without_symlinks(source: &Path, destination: &Path) -> Result<usize, AppError> {
    if !source.exists() {
        return Ok(0);
    }
    fs::create_dir_all(destination).map_err(|e| AppError::io(destination, e))?;
    let mut copied = 0usize;
    for entry in fs::read_dir(source).map_err(|e| AppError::io(source, e))? {
        let entry = entry.map_err(|e| AppError::io(source, e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| AppError::io(entry.path(), e))?;
        if file_type.is_symlink() {
            continue;
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copied += copy_directory_without_symlinks(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).map_err(|e| AppError::io(&target, e))?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_source_database_newer_than_supported() -> Result<(), AppError> {
        let source_dir = tempdir().map_err(|e| AppError::Message(e.to_string()))?;
        let source = Connection::open(source_dir.path().join("cc-switch.db"))?;
        Database::set_user_version(&source, SUPPORTED_SOURCE_SCHEMA_VERSION + 1)?;
        drop(source);

        let destination = Database::memory()?;
        let destination_version_before = {
            let conn = lock_conn!(destination.conn);
            Database::get_user_version(&conn)?
        };
        let error = destination
            .import_from_official_data_dir(source_dir.path())
            .expect_err("newer source must be rejected");
        assert!(error.to_string().contains("最高支持"));
        let conn = lock_conn!(destination.conn);
        assert_eq!(
            Database::get_user_version(&conn)?,
            destination_version_before
        );
        let import_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM x_import_history", [], |row| {
                row.get(0)
            })?;
        assert_eq!(import_count, 0);
        Ok(())
    }

    #[test]
    fn imports_common_rows_and_moves_retry_settings_to_x_table() -> Result<(), AppError> {
        let source_dir = tempdir().map_err(|e| AppError::Message(e.to_string()))?;
        let source = Connection::open(source_dir.path().join("cc-switch.db"))?;
        source.execute_batch(
            "CREATE TABLE providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                PRIMARY KEY (id, app_type)
            );
            CREATE TABLE proxy_config (
                app_type TEXT PRIMARY KEY,
                retry_429_enabled INTEGER NOT NULL,
                retry_429_max_retries INTEGER NOT NULL,
                retry_429_initial_delay_ms INTEGER NOT NULL
            );
            INSERT INTO providers (id, app_type, name, settings_config)
            VALUES ('legacy-provider', 'codex', 'Legacy', '{}');
            INSERT INTO proxy_config
                (app_type, retry_429_enabled, retry_429_max_retries,
                 retry_429_initial_delay_ms)
            VALUES ('codex', 1, 4, 3000);",
        )?;
        Database::set_user_version(&source, 17)?;
        drop(source);

        let destination = Database::memory()?;
        let report = destination.import_from_official_data_dir(source_dir.path())?;

        assert_eq!(report.source_schema_version, 17);
        assert_eq!(report.imported_rows, 1);
        let conn = lock_conn!(destination.conn);
        let provider_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM providers
             WHERE id = 'legacy-provider' AND app_type = 'codex'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(provider_count, 1);
        let retry: (i64, i64, i64) = conn.query_row(
            "SELECT enabled, max_retries, initial_delay_ms
             FROM x_proxy_retry_config WHERE app_type = 'codex'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(retry, (1, 4, 3000));
        Ok(())
    }
}
