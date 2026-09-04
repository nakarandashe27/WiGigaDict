use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf, Prefix};

use uuid::Uuid;
use wigigadict_storage::{ConfigurationRepository, RecoveryRepository};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

#[derive(Clone)]
pub struct ArchiveService {
    database_path: PathBuf,
    managed_root: PathBuf,
}

impl ArchiveService {
    pub fn new(database_path: impl AsRef<Path>, managed_root: impl AsRef<Path>) -> Self {
        Self {
            database_path: database_path.as_ref().to_owned(),
            managed_root: managed_root.as_ref().to_owned(),
        }
    }

    pub fn archive_audio(&self, session_id: &str, started_at: i64, storage_key: &str) {
        let _ = self
            .destination_root()
            .and_then(|root| self.copy_audio_to(&root, session_id, started_at, storage_key));
    }

    pub fn archive_transcript(&self, session_id: &str, content: &str) {
        let result = (|| {
            let root = self.destination_root()?;
            let repository = RecoveryRepository::open(&self.database_path, &self.managed_root)
                .map_err(|error| error.to_string())?;
            let started_at = repository
                .archive_started_at(session_id)
                .map_err(|error| error.to_string())?;
            write_transcript(&root, session_id, started_at, content)
        })();
        let _ = result;
    }

    pub fn backfill_to(&self, root: &Path) -> Result<(), String> {
        let root = prepare_directory(root)?;
        let repository = RecoveryRepository::open(&self.database_path, &self.managed_root)
            .map_err(|error| error.to_string())?;
        let mut first_error = None;
        for entry in repository.list(500).map_err(|error| error.to_string())? {
            match repository
                .archive_audio_storage_key(&entry.session_id)
                .map_err(|error| error.to_string())
            {
                Ok(Some(storage_key)) => {
                    if let Err(error) =
                        self.copy_audio_to(&root, &entry.session_id, entry.started_at, &storage_key)
                    {
                        first_error.get_or_insert(error);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
            if let Some(transcript) = entry.selected
                && let Err(error) = write_transcript(
                    &root,
                    &entry.session_id,
                    entry.started_at,
                    &transcript.content,
                )
            {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub fn backfill_current(&self) -> Result<(), String> {
        let root = self.destination_root()?;
        self.backfill_to(&root)
    }

    fn destination_root(&self) -> Result<PathBuf, String> {
        let configuration = ConfigurationRepository::open(&self.database_path)
            .map_err(|error| error.to_string())?
            .active()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "active configuration is missing".to_owned())?;
        let value = configuration
            .archive_directory
            .ok_or_else(|| "local archive directory is not configured".to_owned())?;
        prepare_directory(Path::new(&value))
    }

    fn copy_audio_to(
        &self,
        root: &Path,
        session_id: &str,
        started_at: i64,
        storage_key: &str,
    ) -> Result<(), String> {
        let source = managed_path(&self.managed_root, storage_key)?;
        let destination = root.join(format!("{}.wav", archive_stem(session_id, started_at)?));
        copy_create_once(&source, &destination).map_err(|error| error.to_string())
    }
}

pub fn default_directory() -> Result<PathBuf, String> {
    // SAFETY: SHGetKnownFolderPath allocates a null-terminated string which is copied before the
    // matching CoTaskMemFree below.
    let value = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) }
        .map_err(|error| format!("Documents folder is unavailable: {error}"))?;
    // SAFETY: value is the valid PWSTR returned above.
    let decoded = unsafe { value.to_string() };
    // SAFETY: SHGetKnownFolderPath documents CoTaskMemFree for this allocation.
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    decoded
        .map(PathBuf::from)
        .map(|path| path.join("WiGigaDict"))
        .map_err(|error| format!("Documents path is not Unicode: {error}"))
}

pub fn prepare_directory(path: &Path) -> Result<PathBuf, String> {
    validate_local_absolute_path(path)?;
    fs::create_dir_all(path)
        .map_err(|error| format!("archive directory cannot be created: {error}"))?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("archive directory is unavailable: {error}"))?;
    let canonical = normalize_windows_path(&canonical);
    validate_local_absolute_path(&canonical)?;

    let probe = canonical.join(format!(".wigigadict-write-test-{}.tmp", Uuid::new_v4()));
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .and_then(|mut file| {
            file.write_all(b"local archive write test")?;
            file.sync_all()
        });
    let _ = fs::remove_file(&probe);
    result.map_err(|error| format!("archive directory is not writable: {error}"))?;
    Ok(canonical)
}

fn normalize_windows_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    value
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_owned())
}

fn validate_local_absolute_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("archive directory must be an absolute local path".into());
    }
    match path.components().next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) => {}
        _ => return Err("network and device paths cannot be used for the local archive".into()),
    }
    Ok(())
}

fn managed_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    let relative = Path::new(key);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("managed audio key is not normalized".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "managed audio root is unavailable")?;
    let source = canonical_root
        .join(relative)
        .canonicalize()
        .map_err(|_| "managed audio file is unavailable")?;
    if !source.starts_with(&canonical_root) {
        return Err("managed audio file escaped its root".into());
    }
    Ok(source)
}

fn archive_stem(session_id: &str, started_at: i64) -> Result<String, String> {
    if started_at < 0
        || session_id.is_empty()
        || !session_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-' || value == b'_')
    {
        return Err("archive identity is invalid".into());
    }
    Ok(format!("dictation-{started_at}-{session_id}"))
}

fn write_transcript(
    root: &Path,
    session_id: &str,
    started_at: i64,
    content: &str,
) -> Result<(), String> {
    let destination = root.join(format!("{}.txt", archive_stem(session_id, started_at)?));
    write_create_once(&destination, content.as_bytes()).map_err(|error| error.to_string())
}

fn copy_create_once(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        return Ok(());
    }
    let temporary = destination.with_extension(format!("wav.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut input = fs::File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_create_once(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    if destination.exists() {
        return Ok(());
    }
    let temporary = destination.with_extension(format!("txt.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_name_is_stable_and_rejects_path_characters() {
        assert_eq!(
            archive_stem("session-1", 42).unwrap(),
            "dictation-42-session-1"
        );
        assert!(archive_stem("../escape", 42).is_err());
        assert!(archive_stem("session-1", -1).is_err());
    }

    #[test]
    fn create_once_does_not_replace_an_existing_transcript() {
        let root = std::env::temp_dir().join(format!("wigigadict-archive-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("record.txt");
        write_create_once(&target, b"first").unwrap();
        write_create_once(&target, b"second").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verbatim_drive_prefix_is_hidden_from_the_owner_facing_path() {
        assert_eq!(
            normalize_windows_path(Path::new(r"\\?\C:\Users\Owner\Documents")),
            PathBuf::from(r"C:\Users\Owner\Documents")
        );
    }
}
