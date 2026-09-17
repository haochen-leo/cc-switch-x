//! 图片 OCR 结果缓存 DAO。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension};

const MEDIA_OCR_CACHE_MAX_ENTRIES: i64 = 512;

impl Database {
    /// 读取 OCR 缓存并刷新最后使用时间。
    pub fn get_media_ocr_cache(&self, cache_key: &str) -> Result<Option<String>, AppError> {
        let now = chrono::Utc::now().timestamp();
        let conn = lock_conn!(self.conn);
        let text = conn
            .query_row(
                "SELECT ocr_text FROM media_ocr_cache WHERE cache_key = ?1",
                [cache_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AppError::Database(error.to_string()))?;

        if text.is_some() {
            conn.execute(
                "UPDATE media_ocr_cache SET last_used_at = ?2 WHERE cache_key = ?1",
                params![cache_key, now],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        }
        Ok(text)
    }

    /// 写入成功的 OCR 结果，并按最近使用时间保留固定数量的记录。
    #[allow(clippy::too_many_arguments)]
    pub fn put_media_ocr_cache(
        &self,
        cache_key: &str,
        image_hash: &str,
        provider_app_type: &str,
        provider_id: &str,
        model: &str,
        prompt_version: &str,
        ocr_text: &str,
    ) -> Result<(), AppError> {
        let now = chrono::Utc::now().timestamp();
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO media_ocr_cache (
                cache_key, image_hash, provider_app_type, provider_id, model,
                prompt_version, ocr_text, created_at, last_used_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(cache_key) DO UPDATE SET
                ocr_text = excluded.ocr_text,
                last_used_at = excluded.last_used_at",
            params![
                cache_key,
                image_hash,
                provider_app_type,
                provider_id,
                model,
                prompt_version,
                ocr_text,
                now,
            ],
        )
        .map_err(|error| AppError::Database(error.to_string()))?;

        conn.execute(
            "DELETE FROM media_ocr_cache
             WHERE cache_key IN (
                SELECT cache_key FROM media_ocr_cache
                ORDER BY last_used_at DESC, created_at DESC, cache_key DESC
                LIMIT -1 OFFSET ?1
             )",
            [MEDIA_OCR_CACHE_MAX_ENTRIES],
        )
        .map_err(|error| AppError::Database(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn test_db() -> Database {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        Database {
            conn: Mutex::new(conn),
        }
    }

    #[test]
    fn media_ocr_cache_round_trip_and_upsert() {
        let db = test_db();
        assert_eq!(db.get_media_ocr_cache("key").unwrap(), None);

        db.put_media_ocr_cache(
            "key",
            "image",
            "codex",
            "provider",
            "vision",
            "v1",
            "第一次",
        )
        .unwrap();
        assert_eq!(
            db.get_media_ocr_cache("key").unwrap().as_deref(),
            Some("第一次")
        );

        db.put_media_ocr_cache(
            "key",
            "image",
            "codex",
            "provider",
            "vision",
            "v1",
            "第二次",
        )
        .unwrap();
        assert_eq!(
            db.get_media_ocr_cache("key").unwrap().as_deref(),
            Some("第二次")
        );
    }

    #[test]
    fn media_ocr_cache_is_bounded() -> Result<(), AppError> {
        let db = test_db();
        for index in 0..=MEDIA_OCR_CACHE_MAX_ENTRIES {
            db.put_media_ocr_cache(
                &format!("key-{index:04}"),
                &format!("image-{index:04}"),
                "codex",
                "provider",
                "vision",
                "v1",
                "text",
            )
            .unwrap();
        }

        let conn = lock_conn!(db.conn);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM media_ocr_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, MEDIA_OCR_CACHE_MAX_ENTRIES);
        Ok(())
    }
}
