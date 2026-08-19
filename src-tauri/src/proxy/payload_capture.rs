use crate::{
    database::Database,
    provider::Provider,
    proxy::{handler_context::RequestContext, server::ProxyState},
};
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
};

const PAYLOAD_LOG_FILE_NAME: &str = "cc-switch-payload.log";
const MB: u64 = 1024 * 1024;

static PAYLOAD_LOG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub(crate) struct PayloadCaptureContext {
    db: Arc<Database>,
    request_id: String,
    tag: &'static str,
    app_type: String,
    endpoint: String,
    provider_id: String,
    provider_name: String,
    session_id: String,
}

impl PayloadCaptureContext {
    pub(crate) fn from_request(state: &ProxyState, ctx: &RequestContext) -> Self {
        Self::from_parts(
            state.db.clone(),
            ctx.request_id.clone(),
            ctx.tag,
            ctx.app_type_str,
            &ctx.endpoint,
            &ctx.session_id,
            &ctx.provider,
        )
    }

    pub(crate) fn from_parts(
        db: Arc<Database>,
        request_id: String,
        tag: &'static str,
        app_type: &str,
        endpoint: &str,
        session_id: &str,
        provider: &Provider,
    ) -> Self {
        Self {
            db,
            request_id,
            tag,
            app_type: app_type.to_string(),
            endpoint: endpoint.to_string(),
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            session_id: session_id.to_string(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.db
            .get_log_config()
            .map(|config| config.enabled && config.capture_payloads)
            .unwrap_or(false)
    }

    pub(crate) fn record_request(&self, body: &[u8]) {
        self.write_record("client_request", None, None, None, body);
    }

    /// 记录发往上游的请求正文；upstream_url 为脱敏后的上游目标 URL
    pub(crate) fn record_upstream_request(&self, upstream_url: &str, body: &[u8]) {
        self.write_record(
            "upstream_request",
            None,
            Some("application/json"),
            Some(upstream_url),
            body,
        );
    }

    pub(crate) fn record_bytes(
        &self,
        phase: &str,
        status: Option<u16>,
        content_type: Option<&str>,
        body: &[u8],
    ) {
        self.write_record(phase, status, content_type, None, body);
    }

    fn write_record(
        &self,
        phase: &str,
        status: Option<u16>,
        content_type: Option<&str>,
        upstream_url: Option<&str>,
        body: &[u8],
    ) {
        // 一次读取配置：开关判断与轮转参数共用，避免多次读库
        let config = match self.db.get_log_config() {
            Ok(config) if config.enabled && config.capture_payloads => config,
            _ => return,
        };

        let byte_len = body.len();
        let body = String::from_utf8_lossy(body);
        let timestamp = chrono::Utc::now().to_rfc3339();
        let status = status
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let content_type = content_type.unwrap_or("-");
        let upstream_url = upstream_url.unwrap_or("-");

        let record = format!(
            "[PAYLOAD] timestamp={timestamp} request_id={} phase={phase} tag={} app={} endpoint={} upstream_url={upstream_url} provider_id={} provider_name={} session_id={} status={status} content_type={content_type} bytes={}\n----- BEGIN PAYLOAD -----\n{}\n----- END PAYLOAD -----\n",
            self.request_id,
            self.tag,
            self.app_type,
            self.endpoint,
            self.provider_id,
            self.provider_name,
            self.session_id,
            byte_len,
            body
        );
        let max_size = config.capture_max_size_mb.max(1) * MB;
        if let Err(error) =
            append_payload_record(&record, max_size, config.capture_archives as usize)
        {
            log::warn!("[Payload] 写入正文日志失败: {error}");
        }
    }
}

fn payload_log_path() -> PathBuf {
    crate::panic_hook::get_log_dir().join(PAYLOAD_LOG_FILE_NAME)
}

fn append_payload_record(
    record: &str,
    max_size: u64,
    archives_to_keep: usize,
) -> std::io::Result<()> {
    let _guard = PAYLOAD_LOG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = payload_log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let current_size = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current_size > 0 && current_size.saturating_add(record.len() as u64) > max_size {
        rotate_payload_log(&path, archives_to_keep)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(record.as_bytes())?;
    file.flush()
}

fn rotate_payload_log(path: &Path, archives_to_keep: usize) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    let mut archive = parent.join(format!("cc-switch-payload_{timestamp}.log"));
    let mut suffix = 1u32;
    while archive.exists() {
        archive = parent.join(format!("cc-switch-payload_{timestamp}_{suffix}.log"));
        suffix = suffix.saturating_add(1);
    }
    fs::rename(path, archive)?;

    let mut archives: Vec<PathBuf> = fs::read_dir(parent)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("cc-switch-payload_") && name.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    archives.sort();
    let keep_from = archives.len().saturating_sub(archives_to_keep);
    for old_archive in archives.into_iter().take(keep_from) {
        // 清理失败不阻塞本次写入，只告警
        if let Err(error) = fs::remove_file(&old_archive) {
            log::warn!(
                "[Payload] 清理旧正文日志失败 {}: {error}",
                old_archive.display()
            );
        }
    }
    Ok(())
}

pub(crate) fn capture_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    context: PayloadCaptureContext,
    phase: &'static str,
    status: u16,
    content_type: Option<String>,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    if !context.enabled() {
        return Box::pin(stream);
    }

    Box::pin(async_stream::stream! {
        let mut body = BytesMut::new();
        let mut stream_error = None;
        tokio::pin!(stream);

        while let Some(item) = stream.next().await {
            match &item {
                Ok(bytes) => body.extend_from_slice(bytes),
                Err(error) => stream_error = Some(error.to_string()),
            }
            yield item;
        }

        context.record_bytes(
            phase,
            Some(status),
            content_type.as_deref(),
            &body,
        );
        if let Some(error) = stream_error {
            log::warn!(
                "[PAYLOAD] request_id={} phase={phase} stream_error={error}",
                context.request_id
            );
        }
    })
}
