//! Ablage der Upload-Bytes hinter einer Abstraktion. Schlüssel ist der
//! SHA-256 des Inhalts: gleiche Dateien liegen nur einmal auf der Platte,
//! URLs bleiben unveränderlich, Backups sind inkrementell billig.
//!
//! Heute gibt es [`LocalFs`] (`{root}/{hex[0..2]}/{hex}`). Ein Object-Store
//! (S3) ließe sich ohne Änderung an API oder Datenbank einhängen.

use std::{
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    time::{Duration, SystemTime},
};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// SHA-256 eines Blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobKey([u8; 32]);

impl BlobKey {
    pub fn new(hash: [u8; 32]) -> Self {
        Self(hash)
    }

    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }

    fn from_hex(text: &str) -> Option<Self> {
        let bytes = hex::decode(text).ok()?;
        Self::from_slice(&bytes)
    }
}

/// Geöffneter Blob-Ausschnitt.
pub struct BlobRead {
    pub body: Pin<Box<dyn AsyncRead + Send>>,
}

pub trait BlobStore: Send + Sync {
    /// Verzeichnis für Zwischendateien während eines Uploads. Liegt auf
    /// demselben Dateisystem wie die Blobs, damit `put_file` atomar umbenennt.
    fn staging_dir(&self) -> &Path;

    /// Fertige Datei unter ihrem Schlüssel übernehmen. `source` ist danach
    /// weg (verschoben oder, falls der Blob schon existiert, gelöscht).
    fn put_file<'a>(&'a self, key: BlobKey, source: &'a Path) -> BoxFuture<'a, io::Result<()>>;

    /// `len` Bytes ab `start` lesen. `None`, wenn es den Blob nicht gibt.
    fn open(
        &self,
        key: BlobKey,
        start: u64,
        len: u64,
    ) -> BoxFuture<'_, io::Result<Option<BlobRead>>>;

    fn delete(&self, key: BlobKey) -> BoxFuture<'_, io::Result<()>>;

    /// Alle Blobs, die seit mindestens `min_age` nicht geschrieben wurden.
    fn list_idle(&self, min_age: Duration) -> BoxFuture<'_, io::Result<Vec<BlobKey>>>;
}

/// Blobs im lokalen Dateisystem.
pub struct LocalFs {
    root: PathBuf,
    staging: PathBuf,
}

impl LocalFs {
    /// Verzeichnisse anlegen und liegengebliebene Zwischendateien entfernen.
    pub async fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        let staging = root.join("tmp");
        tokio::fs::create_dir_all(&staging).await?;
        let store = Self { root, staging };
        store.clean_staging().await;
        Ok(store)
    }

    fn path(&self, key: &BlobKey) -> PathBuf {
        let hex = key.hex();
        self.root.join(&hex[..2]).join(hex)
    }

    /// Zwischendateien älter als einen Tag stammen aus abgebrochenen Uploads.
    async fn clean_staging(&self) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.staging).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let old = entry
                .metadata()
                .await
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(86_400));
            if old {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

fn idle_for(modified: io::Result<SystemTime>, min_age: Duration) -> bool {
    modified
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age >= min_age)
}

impl BlobStore for LocalFs {
    fn staging_dir(&self) -> &Path {
        &self.staging
    }

    fn put_file<'a>(&'a self, key: BlobKey, source: &'a Path) -> BoxFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let target = self.path(&key);
            if tokio::fs::try_exists(&target).await? {
                // Schon vorhanden: Quelle weg, Ziel als „frisch benutzt“ markieren,
                // damit die Bereinigung es nicht gerade jetzt löscht.
                tokio::fs::remove_file(source).await?;
                let target = target.clone();
                tokio::task::spawn_blocking(move || {
                    std::fs::File::options()
                        .append(true)
                        .open(&target)
                        .and_then(|file| file.set_modified(SystemTime::now()))
                })
                .await
                .map_err(io::Error::other)??;
                return Ok(());
            }
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::File::open(source).await?.sync_all().await?;
            tokio::fs::rename(source, &target).await
        })
    }

    fn open(
        &self,
        key: BlobKey,
        start: u64,
        len: u64,
    ) -> BoxFuture<'_, io::Result<Option<BlobRead>>> {
        Box::pin(async move {
            let mut file = match tokio::fs::File::open(self.path(&key)).await {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(err) => return Err(err),
            };
            if start > 0 {
                file.seek(io::SeekFrom::Start(start)).await?;
            }
            Ok(Some(BlobRead {
                body: Box::pin(file.take(len)),
            }))
        })
    }

    fn delete(&self, key: BlobKey) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(async move {
            match tokio::fs::remove_file(self.path(&key)).await {
                Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            }
        })
    }

    fn list_idle(&self, min_age: Duration) -> BoxFuture<'_, io::Result<Vec<BlobKey>>> {
        Box::pin(async move {
            let mut keys = Vec::new();
            let mut shards = tokio::fs::read_dir(&self.root).await?;
            while let Some(shard) = shards.next_entry().await? {
                let name = shard.file_name();
                let Some(name) = name.to_str() else { continue };
                if name.len() != 2 || !shard.file_type().await?.is_dir() {
                    continue;
                }
                let mut files = tokio::fs::read_dir(shard.path()).await?;
                while let Some(file) = files.next_entry().await? {
                    let Some(key) = file.file_name().to_str().and_then(BlobKey::from_hex) else {
                        continue;
                    };
                    let modified = file.metadata().await.and_then(|meta| meta.modified());
                    if idle_for(modified, min_age) {
                        keys.push(key);
                    }
                }
            }
            Ok(keys)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_dedupes_and_reads_ranges() {
        let dir = std::env::temp_dir().join(format!("nova-blob-test-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let store = LocalFs::open(&dir).await.unwrap();
        let key = BlobKey::new([7u8; 32]);

        let first = store.staging_dir().join("a");
        tokio::fs::write(&first, b"hello world").await.unwrap();
        store.put_file(key, &first).await.unwrap();
        assert!(!first.exists());

        let second = store.staging_dir().join("b");
        tokio::fs::write(&second, b"hello world").await.unwrap();
        store.put_file(key, &second).await.unwrap();
        assert!(!second.exists(), "Duplikat wird verworfen");

        let mut read = store.open(key, 6, 5).await.unwrap().unwrap();
        let mut text = String::new();
        read.body.read_to_string(&mut text).await.unwrap();
        assert_eq!(text, "world");

        assert!(
            store
                .list_idle(Duration::from_secs(3600))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.list_idle(Duration::ZERO).await.unwrap(), vec![key]);

        store.delete(key).await.unwrap();
        assert!(store.open(key, 0, 1).await.unwrap().is_none());
        store.delete(key).await.unwrap();
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
