//! Anhänge hochladen, ausliefern, verwerfen und aufräumen.
//!
//! Upload: roher Request-Body (kein Multipart), Dateiname als Query
//! `?filename=`. Der Body wird gestreamt in eine Zwischendatei geschrieben –
//! auch 25 MB belegen keinen RAM –, dabei gezählt und gehasht. Bilder laufen
//! danach durch [`media::strip_image`].
//!
//! Download: nur für Personen, die den Kanal sehen und den Verlauf lesen
//! dürfen (bzw. am DM beteiligt sind). Nicht gesendete Uploads sieht nur, wer
//! sie hochgeladen hat. Antworten tragen `nosniff`, eine Sandbox-CSP und
//! `Content-Disposition: attachment` für alles außer Bildern.

use std::{
    collections::HashSet,
    io::{self, Write},
    path::{Path as FsPath, PathBuf},
    time::Duration,
};

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use futures_util::StreamExt;
use nova_common::permissions::Permissions;
use nova_db::{
    attachments::{NewAttachment, UploadTarget},
    models::{Attachment, CHANNEL_VOICE},
    views,
};
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use tracing::{info, warn};

use crate::{
    AppState,
    blob::BlobKey,
    error::{ApiError, ApiResult},
    guard::visible_channel,
    json_ids,
    media::{self, Kind},
    state::Auth,
};

/// So lange darf ein Upload ohne neue Daten hängen.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Nicht gesendete Uploads bleiben so lange liegen.
const ORPHAN_AGE: chrono::Duration = chrono::Duration::hours(24);
/// Blobs ohne Verweis werden erst nach dieser Ruhezeit gelöscht.
const BLOB_GRACE: Duration = Duration::from_secs(3600);

#[derive(Debug, Deserialize)]
pub(crate) struct UploadQuery {
    filename: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DownloadQuery {
    #[serde(default)]
    download: Option<String>,
}

/// Zwischendatei, die beim Verlassen des Gültigkeitsbereichs gelöscht wird,
/// solange sie nicht übernommen wurde.
struct Staged {
    path: PathBuf,
    keep: bool,
}

impl Staged {
    fn new(dir: &FsPath) -> Self {
        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
        Self {
            path: dir.join(format!("up-{}", hex::encode(random))),
            keep: false,
        }
    }

    fn taken(mut self) {
        self.keep = true;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Schreibt durch und rechnet dabei SHA-256 und Länge mit.
struct HashWriter<W> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Fertig verarbeiteter Upload, bereit für den `BlobStore`.
struct Processed {
    staged: Staged,
    hash: [u8; 32],
    size: u64,
    content_type: &'static str,
    width: Option<i32>,
    height: Option<i32>,
}

pub(crate) async fn upload_to_channel(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (channel, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    if channel.kind == CHANNEL_VOICE {
        return Err(ApiError::BadRequest(
            "voice channels have no messages".into(),
        ));
    }
    // Timeouts entziehen SEND_MESSAGES bereits in `effective_permissions`.
    if !perms.contains(Permissions::SEND_MESSAGES) || !perms.contains(Permissions::ATTACH_FILES) {
        return Err(ApiError::Forbidden);
    }
    if channel.archived {
        return Err(ApiError::Conflict("thread is archived".into()));
    }
    let used = state.db.guild_upload_bytes(channel.guild_id).await?;
    let attachment = receive(
        &state,
        &auth,
        UploadTarget::Channel(channel_id),
        used,
        state.cfg.guild_upload_quota,
        query.filename.as_deref(),
        &headers,
        body,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        json_ids(&views::attachment_view(
            &state.cfg.public_origin,
            &attachment,
        )),
    ))
}

pub(crate) async fn upload_to_dm(
    State(state): State<AppState>,
    auth: Auth,
    Path(dm_id): Path<i64>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if auth.is_bot {
        return Err(ApiError::Forbidden);
    }
    if state.db.dm_peer(dm_id, auth.user_id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let used = state.db.dm_upload_bytes(auth.user_id).await?;
    let attachment = receive(
        &state,
        &auth,
        UploadTarget::Dm(dm_id),
        used,
        state.cfg.dm_upload_quota,
        query.filename.as_deref(),
        &headers,
        body,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        json_ids(&views::attachment_view(
            &state.cfg.public_origin,
            &attachment,
        )),
    ))
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

#[allow(clippy::too_many_arguments)]
async fn receive(
    state: &AppState,
    auth: &Auth,
    target: UploadTarget,
    used: i64,
    quota: u64,
    filename: Option<&str>,
    headers: &HeaderMap,
    body: Body,
) -> ApiResult<Attachment> {
    let per_minute = state.cfg.uploads_per_minute.max(1.0);
    if !state.auth_limiter.allow(
        format!("upload:{}", auth.user_id),
        per_minute,
        per_minute / 60.0,
    ) {
        return Err(ApiError::RateLimited);
    }
    let max = state.cfg.max_upload_bytes as u64;
    let used = u64::try_from(used).unwrap_or(0);
    if content_length(headers).is_some_and(|len| len > max) {
        return Err(ApiError::TooLarge);
    }
    if used.saturating_add(content_length(headers).unwrap_or(1)) > quota {
        return Err(ApiError::QuotaExceeded);
    }
    let filename = media::clean_filename(filename.unwrap_or_default());
    // ID schon beim Start vergeben: Anhänge einer Nachricht stehen dann in der
    // Reihenfolge, in der sie gewählt wurden, nicht in der, in der sie fertig wurden.
    let id = state.ids.next_id();

    let raw = Staged::new(state.blobs.staging_dir());
    let (size, hash, head) = stream_to_file(body, &raw.path, max).await?;
    if size == 0 {
        return Err(ApiError::BadRequest("file is empty".into()));
    }
    if used.saturating_add(size) > quota {
        return Err(ApiError::QuotaExceeded);
    }
    let processed = process(state, raw, size, hash, &head).await?;

    // Bis die Zeile steht, darf die Bereinigung den Blob nicht anfassen.
    let _gc = state.blob_gc.read().await;
    let key = BlobKey::new(processed.hash);
    state
        .blobs
        .put_file(key, &processed.staged.path)
        .await
        .map_err(ApiError::internal)?;
    processed.staged.taken();
    let attachment = state
        .db
        .create_attachment(&NewAttachment {
            id,
            target,
            uploader_id: auth.user_id,
            filename: &filename,
            content_type: processed.content_type,
            size: i64::try_from(processed.size).unwrap_or(i64::MAX),
            sha256: key.as_bytes(),
            width: processed.width,
            height: processed.height,
        })
        .await?;
    state.metrics.uploads.inc();
    Ok(attachment)
}

/// Body in die Zwischendatei streamen. Liefert Größe, Hash und die ersten
/// Bytes für die Typerkennung.
async fn stream_to_file(
    body: Body,
    path: &FsPath,
    max: u64,
) -> ApiResult<(u64, [u8; 32], Vec<u8>)> {
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(ApiError::internal)?;
    let mut stream = body.into_data_stream();
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut head = Vec::with_capacity(media::SNIFF_LEN);
    loop {
        let next = tokio::time::timeout(STALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| ApiError::BadRequest("upload stalled".into()))?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|_| ApiError::BadRequest("upload aborted".into()))?;
        size += chunk.len() as u64;
        if size > max {
            return Err(ApiError::TooLarge);
        }
        if head.len() < media::SNIFF_LEN {
            let take = (media::SNIFF_LEN - head.len()).min(chunk.len());
            head.extend_from_slice(&chunk[..take]);
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(ApiError::internal)?;
    }
    file.flush().await.map_err(ApiError::internal)?;
    Ok((size, hasher.finalize().into(), head))
}

/// Typ bestimmen; Bilder von Metadaten befreien und vermessen.
async fn process(
    state: &AppState,
    raw: Staged,
    size: u64,
    hash: [u8; 32],
    head: &[u8],
) -> ApiResult<Processed> {
    let kind = media::sniff(head);
    let plain = |raw: Staged, content_type: &'static str| Processed {
        staged: raw,
        hash,
        size,
        content_type,
        width: None,
        height: None,
    };
    if !kind.is_image() {
        return Ok(plain(raw, kind.mime()));
    }

    let _permit = state
        .media_limit
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    let cleaned = Staged::new(state.blobs.staging_dir());
    let input = raw.path.clone();
    let output = cleaned.path.clone();
    let result = tokio::task::spawn_blocking(move || strip_file(kind, &input, &output))
        .await
        .map_err(ApiError::internal)?;
    match result {
        Ok((info, hash, size)) if info.inline_ok() => Ok(Processed {
            staged: cleaned,
            hash,
            size,
            content_type: kind.mime(),
            width: i32::try_from(info.width).ok(),
            height: i32::try_from(info.height).ok(),
        }),
        // Riesige Bilder: ohne Metadaten, aber nur als Download.
        Ok((_, hash, size)) => Ok(Processed {
            staged: cleaned,
            hash,
            size,
            content_type: Kind::Other.mime(),
            width: None,
            height: None,
        }),
        // Kaputt oder exotisch: unverändert als Download.
        Err(err) => {
            warn!(error = %err, "image could not be cleaned, storing as file");
            Ok(plain(raw, Kind::Other.mime()))
        }
    }
}

fn strip_file(
    kind: Kind,
    input: &FsPath,
    output: &FsPath,
) -> io::Result<(media::ImageInfo, [u8; 32], u64)> {
    let mut reader = std::fs::File::open(input)?;
    let file = std::fs::File::create(output)?;
    let mut writer = HashWriter {
        inner: io::BufWriter::new(file),
        hasher: Sha256::new(),
        written: 0,
    };
    let info = media::strip_image(kind, &mut reader, &mut writer)?;
    writer.flush()?;
    Ok((info, writer.hasher.finalize().into(), writer.written))
}

// ------------------------------------------------------------- Download

/// Darf `user_id` den Anhang sehen? Sonst 404 (nicht verraten, dass es ihn gibt).
async fn authorize_view(state: &AppState, attachment: &Attachment, user_id: i64) -> ApiResult<()> {
    let pending = attachment.message_id.is_none() && attachment.dm_message_id.is_none();
    let own = attachment.uploader_id == user_id;
    if let Some(channel_id) = attachment.channel_id {
        let (_, perms) = visible_channel(state, channel_id, user_id).await?;
        if pending && !own {
            return Err(ApiError::NotFound);
        }
        if !perms.contains(Permissions::READ_MESSAGE_HISTORY) {
            return Err(ApiError::NotFound);
        }
        return Ok(());
    }
    if let Some(dm_id) = attachment.dm_id {
        if state.db.dm_peer(dm_id, user_id).await?.is_none() || (pending && !own) {
            return Err(ApiError::NotFound);
        }
        return Ok(());
    }
    // Avatare und Server-Icons sieht jede angemeldete Person.
    if own || state.db.attachment_is_public(attachment.id).await? {
        return Ok(());
    }
    Err(ApiError::NotFound)
}

enum ByteRange {
    Full,
    Part { start: u64, end: u64 },
    Unsatisfiable,
}

/// Einfacher `Range`-Header (`bytes=a-b`, `bytes=a-`, `bytes=-n`); mehrere
/// Bereiche werden ignoriert und die ganze Datei geliefert.
fn parse_range(value: Option<&HeaderValue>, size: u64) -> ByteRange {
    let Some(spec) = value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().strip_prefix("bytes="))
    else {
        return ByteRange::Full;
    };
    if spec.contains(',') || size == 0 {
        return ByteRange::Full;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return ByteRange::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    let range = if first.is_empty() {
        match last.parse::<u64>() {
            Ok(0) | Err(_) => return ByteRange::Unsatisfiable,
            Ok(suffix) => (size.saturating_sub(suffix), size - 1),
        }
    } else {
        let Ok(start) = first.parse::<u64>() else {
            return ByteRange::Full;
        };
        let end = if last.is_empty() {
            size - 1
        } else {
            match last.parse::<u64>() {
                Ok(end) => end.min(size - 1),
                Err(_) => return ByteRange::Full,
            }
        };
        (start, end)
    };
    if range.0 >= size || range.0 > range.1 {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Part {
        start: range.0,
        end: range.1,
    }
}

fn header_value(text: &str) -> HeaderValue {
    HeaderValue::from_str(text).unwrap_or_else(|_| HeaderValue::from_static("invalid"))
}

pub(crate) async fn download(
    State(state): State<AppState>,
    auth: Auth,
    Path(attachment_id): Path<i64>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let Some(attachment) = state.db.attachment(attachment_id).await? else {
        return Err(ApiError::NotFound);
    };
    authorize_view(&state, &attachment, auth.user_id).await?;
    let Some(key) = BlobKey::from_slice(&attachment.sha256) else {
        return Err(ApiError::NotFound);
    };
    let etag = format!("\"{}\"", key.hex());
    let size = u64::try_from(attachment.size).unwrap_or(0);
    let inline = attachment.content_type.starts_with("image/") && query.download.is_none();

    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::ETAG, header_value(&etag));
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|tag| tag.trim() == etag));
    if not_modified {
        return Ok((StatusCode::NOT_MODIFIED, response_headers).into_response());
    }

    let (status, start, len) = match parse_range(headers.get(header::RANGE), size) {
        ByteRange::Full => (StatusCode::OK, 0, size),
        ByteRange::Part { start, end } => {
            response_headers.insert(
                header::CONTENT_RANGE,
                header_value(&format!("bytes {start}-{end}/{size}")),
            );
            (StatusCode::PARTIAL_CONTENT, start, end - start + 1)
        }
        ByteRange::Unsatisfiable => {
            response_headers.insert(
                header::CONTENT_RANGE,
                header_value(&format!("bytes */{size}")),
            );
            return Ok((StatusCode::RANGE_NOT_SATISFIABLE, response_headers).into_response());
        }
    };
    let Some(blob) = state
        .blobs
        .open(key, start, len)
        .await
        .map_err(ApiError::internal)?
    else {
        warn!(attachment_id, "blob missing for attachment");
        return Err(ApiError::NotFound);
    };

    response_headers.insert(header::CONTENT_TYPE, header_value(&attachment.content_type));
    response_headers.insert(header::CONTENT_LENGTH, header_value(&len.to_string()));
    response_headers.insert(
        header::CONTENT_DISPOSITION,
        header_value(&media::content_disposition(inline, &attachment.filename)),
    );
    let body = Body::from_stream(ReaderStream::new(blob.body));
    Ok((status, response_headers, body).into_response())
}

/// Eigenen, noch nicht gesendeten Upload verwerfen (Chip im Composer entfernt).
pub(crate) async fn discard(
    State(state): State<AppState>,
    auth: Auth,
    Path(attachment_id): Path<i64>,
) -> ApiResult<StatusCode> {
    let removed = state
        .db
        .delete_pending_attachment(attachment_id, auth.user_id)
        .await?;
    if removed == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------ Aufräumen

/// Alle 15 Minuten: verwaiste Uploads löschen, dann Blobs ohne Verweis.
pub fn spawn_janitor(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(900));
        loop {
            tick.tick().await;
            if let Err(err) = sweep(&state).await {
                warn!(error = %err, "upload cleanup failed");
            }
        }
    });
}

/// Eine Aufräumrunde; öffentlich für Tests.
pub async fn sweep(state: &AppState) -> anyhow::Result<(u64, usize)> {
    let rows = state
        .db
        .delete_orphan_attachments(Utc::now() - ORPHAN_AGE)
        .await?;
    let candidates = state.blobs.list_idle(BLOB_GRACE).await?;
    let mut deleted = 0;
    for chunk in candidates.chunks(500) {
        let hashes: Vec<Vec<u8>> = chunk.iter().map(|key| key.as_bytes().to_vec()).collect();
        // Exklusiv: kein Upload darf zwischen Prüfung und Löschen einen
        // Verweis auf einen dieser Blobs anlegen.
        let _gc = state.blob_gc.write().await;
        let referenced: HashSet<Vec<u8>> = state
            .db
            .referenced_blobs(&hashes)
            .await?
            .into_iter()
            .collect();
        for key in chunk {
            if !referenced.contains(key.as_bytes().as_slice()) {
                state.blobs.delete(*key).await?;
                deleted += 1;
            }
        }
    }
    if rows > 0 || deleted > 0 {
        info!(rows, blobs = deleted, "upload cleanup");
    }
    Ok((rows, deleted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(value: &str, size: u64) -> (bool, u64, u64) {
        match parse_range(Some(&HeaderValue::from_str(value).unwrap()), size) {
            ByteRange::Full => (true, 0, size),
            ByteRange::Part { start, end } => (false, start, end),
            ByteRange::Unsatisfiable => (false, u64::MAX, u64::MAX),
        }
    }

    #[test]
    fn ranges() {
        assert_eq!(range("bytes=0-99", 1000), (false, 0, 99));
        assert_eq!(range("bytes=900-", 1000), (false, 900, 999));
        assert_eq!(range("bytes=-100", 1000), (false, 900, 999));
        assert_eq!(range("bytes=0-5000", 1000), (false, 0, 999));
        assert_eq!(range("bytes=-5000", 1000), (false, 0, 999));
        assert_eq!(range("bytes=1000-", 1000), (false, u64::MAX, u64::MAX));
        assert_eq!(range("bytes=5-1", 1000), (false, u64::MAX, u64::MAX));
        assert_eq!(range("bytes=0-1,5-6", 1000), (true, 0, 1000));
        assert_eq!(range("items=0-1", 1000), (true, 0, 1000));
        assert!(matches!(parse_range(None, 10), ByteRange::Full));
    }
}
