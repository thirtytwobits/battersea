//! Copyright (c) Scott A Dixon
//! Application-owned file storage for one provider conversation.
use async_trait::async_trait;
use battersea_model::{ConversationCheckpoint, ConversationStore, ConversationStoreError};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// The host owns the directory and fences old executors before removing a crash-left lock.
/// File replacement and directory synchronisation require a filesystem with durable rename.
pub struct FileConversationStore {
    directory: PathBuf,
}
impl FileConversationStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
}
fn unavailable(_: impl std::fmt::Debug) -> ConversationStoreError {
    ConversationStoreError::Unavailable
}
fn read(
    directory: &Path,
    run_id: &str,
    max: usize,
) -> Result<Option<ConversationCheckpoint>, ConversationStoreError> {
    let file = match File::open(directory.join("conversation.json")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(unavailable(e)),
    };
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(unavailable)?;
    if bytes.len() > max {
        return Err(ConversationStoreError::Unavailable);
    }
    let record: ConversationCheckpoint = serde_json::from_slice(&bytes).map_err(unavailable)?;
    if record.scope.run_id != run_id {
        return Err(ConversationStoreError::Conflict);
    }
    Ok(Some(record))
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[async_trait]
impl ConversationStore for FileConversationStore {
    async fn load(
        &self,
        run_id: &str,
        max_bytes: usize,
    ) -> Result<Option<ConversationCheckpoint>, ConversationStoreError> {
        let directory = self.directory.clone();
        let run_id = run_id.to_owned();
        tokio::task::spawn_blocking(move || read(&directory, &run_id, max_bytes))
            .await
            .map_err(unavailable)?
    }
    async fn compare_exchange(
        &self,
        run_id: &str,
        expected_revision: Option<u64>,
        next: &ConversationCheckpoint,
    ) -> Result<(), ConversationStoreError> {
        next.validate().map_err(unavailable)?;
        if next.scope.run_id != run_id
            || expected_revision.map_or(Some(1), |r| r.checked_add(1)) != Some(next.revision)
        {
            return Err(ConversationStoreError::Conflict);
        }
        let directory = self.directory.clone();
        let run_id = run_id.to_owned();
        let next = next.clone();
        tokio::task::spawn_blocking(move || {
            let lock_path = directory.join("conversation.lock");
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        ConversationStoreError::Conflict
                    } else {
                        unavailable(e)
                    }
                })?;
            let _lock = Lock(lock_path);
            let previous = read(&directory, &run_id, next.max_bytes)?;
            if previous.as_ref().map(|r| r.revision) != expected_revision {
                return Err(ConversationStoreError::Conflict);
            }
            let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(unavailable)?;
            serde_json::to_writer(&mut temporary, &next).map_err(unavailable)?;
            temporary.flush().map_err(unavailable)?;
            temporary.as_file().sync_all().map_err(unavailable)?;
            temporary
                .persist(directory.join("conversation.json"))
                .map_err(unavailable)?;
            File::open(&directory)
                .and_then(|d| d.sync_all())
                .map_err(unavailable)?;
            Ok(())
        })
        .await
        .map_err(unavailable)?
    }
}
