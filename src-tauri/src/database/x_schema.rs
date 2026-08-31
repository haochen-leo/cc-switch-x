use super::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::Connection;

pub(crate) const X_SCHEMA_VERSION: i32 = 2;

impl Database {
    pub(crate) fn apply_x_schema(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::apply_x_schema_on_conn(&conn)
    }

    pub(crate) fn apply_x_schema_on_conn(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS x_schema_meta (
                component TEXT PRIMARY KEY,
                version INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS x_proxy_retry_config (
                app_type TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 1,
                max_retries INTEGER NOT NULL DEFAULT 2,
                initial_delay_ms INTEGER NOT NULL DEFAULT 2000,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS x_import_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_path TEXT NOT NULL,
                source_schema_version INTEGER NOT NULL,
                imported_at TEXT NOT NULL DEFAULT (datetime('now')),
                imported_rows INTEGER NOT NULL DEFAULT 0
            );",
        )
        .map_err(|e| AppError::Database(format!("创建 CC Switch X 扩展表失败: {e}")))?;

        let mut current_version: i32 = conn
            .query_row(
                "SELECT version FROM x_schema_meta WHERE component = 'cc-switch-x'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if current_version > X_SCHEMA_VERSION {
            return Err(AppError::Database(format!(
                "CC Switch X 扩展数据库版本过新（{current_version}），当前仅支持 {X_SCHEMA_VERSION}"
            )));
        }

        if current_version < 1 {
            for app_type in ["claude", "codex", "gemini", "grokbuild"] {
                conn.execute(
                    "INSERT OR IGNORE INTO x_proxy_retry_config (app_type)
                     VALUES (?1)",
                    [app_type],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
            }

            // 官方表结构保持不变，仅把 X 自己数据库中的初始端口数据改为独立端口。
            conn.execute(
                "UPDATE proxy_config SET listen_port = ?1 WHERE listen_port = 15721",
                [i64::from(crate::brand::DEFAULT_PROXY_PORT)],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

            conn.execute(
                "INSERT OR REPLACE INTO x_schema_meta (component, version)
                 VALUES ('cc-switch-x', ?1)",
                [1],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            current_version = 1;
        }

        if current_version < 2 {
            // 兼容早期本地分支：429 重试字段曾被放进官方 proxy_config。
            // 只在 X schema v1 -> v2 时迁移一次，之后运行层完全使用 x_ 表。
            let has_legacy_retry_columns = [
                "retry_429_enabled",
                "retry_429_max_retries",
                "retry_429_initial_delay_ms",
            ]
            .into_iter()
            .map(|column| Self::has_column(conn, "proxy_config", column))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .all(|present| present);

            if has_legacy_retry_columns {
                conn.execute_batch(
                    "INSERT INTO x_proxy_retry_config
                        (app_type, enabled, max_retries, initial_delay_ms, updated_at)
                     SELECT app_type, retry_429_enabled, retry_429_max_retries,
                            retry_429_initial_delay_ms, datetime('now')
                     FROM proxy_config
                     WHERE true
                     ON CONFLICT(app_type) DO UPDATE SET
                        enabled = excluded.enabled,
                        max_retries = excluded.max_retries,
                        initial_delay_ms = excluded.initial_delay_ms,
                        updated_at = datetime('now');",
                )
                .map_err(|e| {
                    AppError::Database(format!("迁移旧版 429 重试配置到 X 扩展表失败: {e}"))
                })?;
            }

            conn.execute(
                "INSERT OR REPLACE INTO x_schema_meta (component, version)
                 VALUES ('cc-switch-x', ?1)",
                [2],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn x_schema_does_not_change_official_user_version() -> Result<(), AppError> {
        let conn = Connection::open_in_memory()?;
        Database::create_tables_on_conn(&conn)?;
        Database::set_user_version(&conn, super::super::SCHEMA_VERSION)?;

        Database::apply_x_schema_on_conn(&conn)?;

        assert_eq!(
            Database::get_user_version(&conn)?,
            super::super::SCHEMA_VERSION
        );
        let port: i64 = conn.query_row(
            "SELECT listen_port FROM proxy_config WHERE app_type = 'claude'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(port, i64::from(crate::brand::DEFAULT_PROXY_PORT));
        for column in [
            "retry_429_enabled",
            "retry_429_max_retries",
            "retry_429_initial_delay_ms",
        ] {
            assert!(!Database::has_column(&conn, "proxy_config", column)?);
        }
        let x_version: i64 = conn.query_row(
            "SELECT version FROM x_schema_meta WHERE component = 'cc-switch-x'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(x_version, i64::from(X_SCHEMA_VERSION));
        Ok(())
    }

    #[test]
    fn x_schema_migrates_legacy_retry_columns_without_changing_user_version() -> Result<(), AppError>
    {
        let conn = Connection::open_in_memory()?;
        Database::create_tables_on_conn(&conn)?;
        Database::set_user_version(&conn, super::super::SCHEMA_VERSION)?;
        conn.execute_batch(
            "ALTER TABLE proxy_config
                ADD COLUMN retry_429_enabled INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE proxy_config
                ADD COLUMN retry_429_max_retries INTEGER NOT NULL DEFAULT 2;
             ALTER TABLE proxy_config
                ADD COLUMN retry_429_initial_delay_ms INTEGER NOT NULL DEFAULT 2000;
             UPDATE proxy_config
             SET retry_429_enabled = 0,
                 retry_429_max_retries = 5,
                 retry_429_initial_delay_ms = 3500
             WHERE app_type = 'codex';
             CREATE TABLE x_schema_meta (
                component TEXT PRIMARY KEY,
                version INTEGER NOT NULL
             );
             INSERT INTO x_schema_meta (component, version)
             VALUES ('cc-switch-x', 1);",
        )?;

        Database::apply_x_schema_on_conn(&conn)?;

        assert_eq!(
            Database::get_user_version(&conn)?,
            super::super::SCHEMA_VERSION
        );
        let retry: (i64, i64, i64) = conn.query_row(
            "SELECT enabled, max_retries, initial_delay_ms
             FROM x_proxy_retry_config WHERE app_type = 'codex'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(retry, (0, 5, 3500));
        Ok(())
    }
}
