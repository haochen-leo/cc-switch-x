//! Codex rollout 文件的两种形态：`rollout-*.jsonl` 与压缩后的 `rollout-*.jsonl.zst`。
//!
//! Codex 的 `[features] local_thread_store_compression`（以及不受该开关控制的
//! app-server `rollout/compress`）会把 7 天没动过的 rollout 压成同名 `.jsonl.zst`
//! （单帧 zstd、保留 mtime）并删掉原文件；在压缩过的会话上继续对话时，Codex
//! 先解压回 `.jsonl` 再追加。会话管理、用量导入、历史分桶迁移都经这里读写，
//! 两种形态对它们透明。

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use crate::config::atomic_write;
use crate::error::AppError;

const COMPRESSED_SUFFIX: &str = ".zst";
const PLAIN_SUFFIX: &str = ".jsonl";
const READER_CAPACITY: usize = 1 << 20;
/// 与 Codex 压缩时用的级别无关，只用于 CC Switch 原地改写压缩文件（历史分桶迁移）。
const WRITE_LEVEL: i32 = 3;

fn file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|name| name.to_str())
}

/// 是否为压缩形态（`*.jsonl.zst`）。
pub fn is_compressed(path: &Path) -> bool {
    file_name(path).is_some_and(|name| {
        name.strip_suffix(COMPRESSED_SUFFIX)
            .is_some_and(|plain| plain.ends_with(PLAIN_SUFFIX))
    })
}

/// 是否为 rollout 文件（`*.jsonl` 或 `*.jsonl.zst`），只看扩展名。
pub fn is_rollout_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") || is_compressed(path)
}

/// 按行读取，压缩文件透明解压。
pub fn open_reader(path: &Path) -> io::Result<Box<dyn BufRead + Send>> {
    reader_from_file(path, File::open(path)?)
}

/// 同 [`open_reader`]，用调用方已打开的文件（调用方还要从同一句柄取元数据时用）。
pub fn reader_from_file(path: &Path, file: File) -> io::Result<Box<dyn BufRead + Send>> {
    if is_compressed(path) {
        let decoder = zstd::stream::read::Decoder::new(file)?;
        Ok(Box::new(BufReader::with_capacity(READER_CAPACITY, decoder)))
    } else {
        Ok(Box::new(BufReader::with_capacity(READER_CAPACITY, file)))
    }
}

/// 读出全文（UTF-8），压缩文件先解压。
pub fn read_to_string(path: &Path) -> io::Result<String> {
    if !is_compressed(path) {
        return std::fs::read_to_string(path);
    }
    let mut text = String::new();
    open_reader(path)?.read_to_string(&mut text)?;
    Ok(text)
}

/// 原子写回；压缩路径先编码成 zstd，保持文件原有形态。
///
/// 两种形态都保留原 mtime：`codex resume` 按 mtime 排序，会话列表里压缩会话的最后活跃
/// 时间也取 mtime（Codex 压缩时也保留了它），改写元数据不该让老会话显示成刚刚活跃。
pub fn write_rollout(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok();
    if is_compressed(path) {
        let encoded = zstd::stream::encode_all(bytes, WRITE_LEVEL)
            .map_err(|error| AppError::io(path, error))?;
        atomic_write(path, &encoded)?;
    } else {
        atomic_write(path, bytes)?;
    }
    if let Some(modified) = modified {
        // Unix 设时间只看属主，只读打开就够（文件本身可能只读）；Windows 要可写句柄。
        // 内容已经写好，时间戳只影响排序：恢复失败不该让整次迁移中断
        let file = if cfg!(windows) {
            File::options().write(true).open(path)
        } else {
            File::open(path)
        };
        let times = std::fs::FileTimes::new().set_modified(modified);
        if let Err(error) = file.and_then(|file| file.set_times(times)) {
            log::warn!("Failed to restore mtime of {}: {error}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_both_forms() {
        let plain = Path::new("/x/rollout-a.jsonl");
        let zst = Path::new("/x/rollout-a.jsonl.zst");
        assert!(is_rollout_file(plain));
        assert!(is_rollout_file(zst));
        assert!(!is_compressed(plain));
        assert!(is_compressed(zst));
        assert!(!is_rollout_file(Path::new("/x/rollout-a.zst")));
        assert!(!is_rollout_file(Path::new("/x/notes.txt")));
        assert!(!is_compressed(Path::new("/x/rollout-a.zst")));
    }

    #[test]
    fn rewriting_rollout_keeps_mtime() {
        let temp = tempfile::tempdir().expect("tempdir");
        for name in ["rollout-a.jsonl.zst", "rollout-b.jsonl"] {
            let path = temp.path().join(name);
            write_rollout(&path, b"{\"a\":1}\n").expect("write");
            let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
            File::options()
                .write(true)
                .open(&path)
                .expect("open")
                .set_times(std::fs::FileTimes::new().set_modified(old))
                .expect("set mtime");

            write_rollout(&path, b"{\"a\":2}\n").expect("rewrite");

            let modified = std::fs::metadata(&path)
                .expect("meta")
                .modified()
                .expect("mtime");
            assert_eq!(modified, old, "{name}");
            assert_eq!(read_to_string(&path).expect("read"), "{\"a\":2}\n");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rewriting_read_only_rollout_keeps_mtime() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().expect("tempdir");
        for name in ["rollout-c.jsonl", "rollout-d.jsonl.zst"] {
            let path = temp.path().join(name);
            write_rollout(&path, b"{\"a\":1}\n").expect("write");
            let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
            File::options()
                .write(true)
                .open(&path)
                .expect("open")
                .set_times(std::fs::FileTimes::new().set_modified(old))
                .expect("set mtime");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).expect("chmod");

            write_rollout(&path, b"{\"a\":2}\n").expect("rewrite");

            let modified = std::fs::metadata(&path)
                .expect("meta")
                .modified()
                .expect("mtime");
            assert_eq!(
                read_to_string(&path).expect("read"),
                "{\"a\":2}\n",
                "{name}"
            );
            assert_eq!(modified, old, "{name}");
        }
    }

    #[test]
    fn read_and_write_round_trip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let zst = temp.path().join("rollout-a.jsonl.zst");
        let plain = temp.path().join("rollout-b.jsonl");
        let text = "{\"a\":1}\n{\"b\":2}\n";

        write_rollout(&zst, text.as_bytes()).expect("write zst");
        write_rollout(&plain, text.as_bytes()).expect("write plain");

        let raw = std::fs::read(&zst).expect("raw");
        assert_eq!(&raw[..4], &[0x28, 0xB5, 0x2F, 0xFD], "zstd magic");
        assert_eq!(read_to_string(&zst).expect("read zst"), text);
        assert_eq!(read_to_string(&plain).expect("read plain"), text);
        let lines: Vec<String> = open_reader(&zst)
            .expect("reader")
            .lines()
            .collect::<Result<_, _>>()
            .expect("lines");
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
    }
}
