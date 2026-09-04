use std::fmt::{Display, Formatter};
use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{Database, StorageError};

pub const CONFIGURATION_SCHEMA_VERSION: u32 = 2;
pub const DEFAULT_HOTKEY_BINDING: &str = "F8";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfiguration {
    pub config_version: u32,
    pub hotkey_binding: String,
    pub microphone_device_id: Option<String>,
    pub active_runtime_profile_id: Option<String>,
    pub active_cleanup_profile_id: Option<String>,
    pub startup_enabled: bool,
    pub warmup_enabled: bool,
    pub diagnostic_mode: bool,
    pub archive_directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationUpdate {
    pub expected_config_version: u32,
    pub hotkey_binding: String,
    pub microphone_device_id: Option<String>,
    pub active_runtime_profile_id: Option<String>,
    pub active_cleanup_profile_id: Option<String>,
    pub startup_enabled: bool,
    pub warmup_enabled: bool,
    pub diagnostic_mode: bool,
    pub archive_directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProfileOption {
    pub id: String,
    pub model_name: String,
    pub model_version: String,
    pub device_kind: String,
    pub health_state: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupProfileOption {
    pub id: String,
    pub name: String,
    pub profile_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationCatalog {
    pub configuration: AppConfiguration,
    pub runtime_profiles: Vec<RuntimeProfileOption>,
    pub cleanup_profiles: Vec<CleanupProfileOption>,
}

#[derive(Debug)]
pub enum ConfigurationRepositoryError {
    Storage(StorageError),
    Sqlite(rusqlite::Error),
    Invalid(String),
    Conflict(String),
}

impl Display for ConfigurationRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "configuration database error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid configuration: {message}"),
            Self::Conflict(message) => write!(formatter, "configuration conflict: {message}"),
        }
    }
}

impl std::error::Error for ConfigurationRepositoryError {}

impl From<StorageError> for ConfigurationRepositoryError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<rusqlite::Error> for ConfigurationRepositoryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type ConfigurationRepositoryResult<T> = std::result::Result<T, ConfigurationRepositoryError>;

pub struct ConfigurationRepository {
    database: Database,
}

impl ConfigurationRepository {
    pub fn open(path: impl AsRef<Path>) -> ConfigurationRepositoryResult<Self> {
        Ok(Self {
            database: Database::open(path)?,
        })
    }

    pub fn open_in_memory() -> ConfigurationRepositoryResult<Self> {
        Ok(Self {
            database: Database::open_in_memory()?,
        })
    }

    pub fn active(&self) -> ConfigurationRepositoryResult<Option<AppConfiguration>> {
        self.database
            .connection
            .query_row(
                "SELECT config_version,hotkey_binding,microphone_device_id,
                        active_runtime_profile_id,active_cleanup_profile_id,
                        startup_enabled,warmup_enabled,diagnostic_mode,archive_directory
                 FROM app_configuration WHERE is_active=1",
                [],
                map_configuration,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn ensure_default(
        &mut self,
        observed_at: i64,
    ) -> ConfigurationRepositoryResult<AppConfiguration> {
        validate_observed_at(observed_at)?;
        if let Some(active) = self.active()? {
            return Ok(active);
        }
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(active) = transaction
            .query_row(
                "SELECT config_version,hotkey_binding,microphone_device_id,
                        active_runtime_profile_id,active_cleanup_profile_id,
                        startup_enabled,warmup_enabled,diagnostic_mode,archive_directory
                 FROM app_configuration WHERE is_active=1",
                [],
                map_configuration,
            )
            .optional()?
        {
            transaction.commit()?;
            return Ok(active);
        }
        transaction.execute(
            "INSERT INTO app_configuration(
                id,schema_version,config_version,is_active,hotkey_binding,microphone_device_id,
                active_runtime_profile_id,active_cleanup_profile_id,startup_enabled,warmup_enabled,
                diagnostic_mode,archive_directory,created_at,activated_at,superseded_at
             ) VALUES(?1,?2,1,1,?3,NULL,NULL,NULL,0,0,0,NULL,?4,?4,NULL)",
            params![
                format!("default-config-{observed_at}"),
                CONFIGURATION_SCHEMA_VERSION,
                DEFAULT_HOTKEY_BINDING,
                observed_at,
            ],
        )?;
        transaction.commit()?;
        self.active()?.ok_or_else(|| {
            ConfigurationRepositoryError::Conflict("default snapshot was not activated".into())
        })
    }

    pub fn catalog(&self) -> ConfigurationRepositoryResult<ConfigurationCatalog> {
        let configuration = self.active()?.ok_or_else(|| {
            ConfigurationRepositoryError::Conflict("active snapshot is missing".into())
        })?;
        let mut runtime_query = self.database.connection.prepare(
            "SELECT p.id,m.model_name,m.model_version,p.device_kind,p.health_state,
                    (p.enabled=1 AND p.health_state='healthy' AND m.install_state='installed')
             FROM runtime_profile p
             JOIN model_package m ON m.id=p.model_package_id
             ORDER BY m.model_name,p.device_kind,p.profile_version DESC",
        )?;
        let runtime_profiles = runtime_query
            .query_map([], |row| {
                Ok(RuntimeProfileOption {
                    id: row.get(0)?,
                    model_name: row.get(1)?,
                    model_version: row.get(2)?,
                    device_kind: row.get(3)?,
                    health_state: row.get(4)?,
                    available: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut cleanup_query = self.database.connection.prepare(
            "SELECT id,name,profile_version
             FROM cleanup_profile
             WHERE enabled=1 AND superseded_at IS NULL
             ORDER BY name,profile_version DESC",
        )?;
        let cleanup_profiles = cleanup_query
            .query_map([], |row| {
                Ok(CleanupProfileOption {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    profile_version: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ConfigurationCatalog {
            configuration,
            runtime_profiles,
            cleanup_profiles,
        })
    }

    pub fn validate_update(
        &self,
        update: &ConfigurationUpdate,
    ) -> ConfigurationRepositoryResult<()> {
        validate_update_shape(update)?;
        let active = self.active()?.ok_or_else(|| {
            ConfigurationRepositoryError::Conflict("active snapshot is missing".into())
        })?;
        if active.config_version != update.expected_config_version {
            return Err(ConfigurationRepositoryError::Conflict(
                "active snapshot changed; reload settings".into(),
            ));
        }
        validate_profile_references(&self.database.connection, update)
    }

    pub fn update(
        &mut self,
        update: &ConfigurationUpdate,
        observed_at: i64,
    ) -> ConfigurationRepositoryResult<AppConfiguration> {
        validate_observed_at(observed_at)?;
        validate_update_shape(update)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active_version = transaction
            .query_row(
                "SELECT config_version FROM app_configuration WHERE is_active=1",
                [],
                |row| row.get::<_, u32>(0),
            )
            .optional()?;
        if active_version != Some(update.expected_config_version) {
            return Err(ConfigurationRepositoryError::Conflict(
                "active snapshot changed; reload settings".into(),
            ));
        }
        validate_profile_references(&transaction, update)?;
        let next_version: u32 = transaction.query_row(
            "SELECT COALESCE(MAX(config_version),0)+1 FROM app_configuration",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE app_configuration
             SET is_active=0,superseded_at=?1
             WHERE is_active=1 AND config_version=?2",
            params![observed_at, update.expected_config_version],
        )?;
        transaction.execute(
            "INSERT INTO app_configuration(
                id,schema_version,config_version,is_active,hotkey_binding,microphone_device_id,
                active_runtime_profile_id,active_cleanup_profile_id,startup_enabled,warmup_enabled,
                diagnostic_mode,archive_directory,created_at,activated_at,superseded_at
             ) VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12,NULL)",
            params![
                format!("settings-config-{next_version}-{observed_at}"),
                CONFIGURATION_SCHEMA_VERSION,
                next_version,
                update.hotkey_binding,
                update.microphone_device_id,
                update.active_runtime_profile_id,
                update.active_cleanup_profile_id,
                update.startup_enabled,
                update.warmup_enabled,
                update.diagnostic_mode,
                update.archive_directory,
                observed_at,
            ],
        )?;
        transaction.commit()?;
        self.active()?.ok_or_else(|| {
            ConfigurationRepositoryError::Conflict("new snapshot was not activated".into())
        })
    }
}

fn map_configuration(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppConfiguration> {
    Ok(AppConfiguration {
        config_version: row.get(0)?,
        hotkey_binding: row.get(1)?,
        microphone_device_id: row.get(2)?,
        active_runtime_profile_id: row.get(3)?,
        active_cleanup_profile_id: row.get(4)?,
        startup_enabled: row.get(5)?,
        warmup_enabled: row.get(6)?,
        diagnostic_mode: row.get(7)?,
        archive_directory: row.get(8)?,
    })
}

fn validate_observed_at(observed_at: i64) -> ConfigurationRepositoryResult<()> {
    if observed_at < 0 {
        return Err(ConfigurationRepositoryError::Invalid(
            "observed_at must be non-negative".into(),
        ));
    }
    Ok(())
}

fn validate_update_shape(update: &ConfigurationUpdate) -> ConfigurationRepositoryResult<()> {
    if update.expected_config_version == 0 {
        return Err(ConfigurationRepositoryError::Invalid(
            "expected_config_version must be positive".into(),
        ));
    }
    if update.hotkey_binding.is_empty() || update.hotkey_binding.len() > 64 {
        return Err(ConfigurationRepositoryError::Invalid(
            "hotkey_binding must contain 1..64 characters".into(),
        ));
    }
    if update
        .microphone_device_id
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 512)
    {
        return Err(ConfigurationRepositoryError::Invalid(
            "microphone_device_id must contain 1..512 characters".into(),
        ));
    }
    if update.warmup_enabled && update.active_runtime_profile_id.is_none() {
        return Err(ConfigurationRepositoryError::Invalid(
            "warm-up requires an active runtime profile".into(),
        ));
    }
    if update
        .archive_directory
        .as_ref()
        .is_some_and(|value| value.len() < 3 || value.len() > 1024 || value.trim() != value)
    {
        return Err(ConfigurationRepositoryError::Invalid(
            "archive_directory must contain 3..1024 trimmed characters".into(),
        ));
    }
    Ok(())
}

fn validate_profile_references(
    connection: &rusqlite::Connection,
    update: &ConfigurationUpdate,
) -> ConfigurationRepositoryResult<()> {
    if let Some(profile_id) = update.active_runtime_profile_id.as_deref() {
        let ready = connection
            .query_row(
                "SELECT 1
                 FROM runtime_profile p
                 JOIN model_package m ON m.id=p.model_package_id
                 WHERE p.id=?1 AND p.enabled=1 AND p.health_state='healthy'
                   AND m.install_state='installed'",
                [profile_id],
                |_| Ok(()),
            )
            .optional()?;
        if ready.is_none() {
            return Err(ConfigurationRepositoryError::Invalid(
                "runtime profile is not installed, enabled, and healthy".into(),
            ));
        }
    }
    if let Some(profile_id) = update.active_cleanup_profile_id.as_deref() {
        let enabled = connection
            .query_row(
                "SELECT 1 FROM cleanup_profile
                 WHERE id=?1 AND enabled=1 AND superseded_at IS NULL",
                [profile_id],
                |_| Ok(()),
            )
            .optional()?;
        if enabled.is_none() {
            return Err(ConfigurationRepositoryError::Invalid(
                "cleanup profile is not enabled".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update_from(configuration: &AppConfiguration) -> ConfigurationUpdate {
        ConfigurationUpdate {
            expected_config_version: configuration.config_version,
            hotkey_binding: "control+shift+Space".into(),
            microphone_device_id: Some("microphone-1".into()),
            active_runtime_profile_id: None,
            active_cleanup_profile_id: None,
            startup_enabled: true,
            warmup_enabled: false,
            diagnostic_mode: true,
            archive_directory: Some(r"C:\Users\Owner\Documents\WiGigaDict".into()),
        }
    }

    #[test]
    fn default_and_updates_are_versioned_immutable_snapshots() {
        let mut repository = ConfigurationRepository::open_in_memory().unwrap();
        let default = repository.ensure_default(10).unwrap();
        assert_eq!(default.config_version, 1);
        assert_eq!(default.hotkey_binding, DEFAULT_HOTKEY_BINDING);

        let saved = repository.update(&update_from(&default), 20).unwrap();
        assert_eq!(saved.config_version, 2);
        assert_eq!(saved.microphone_device_id.as_deref(), Some("microphone-1"));
        assert!(saved.startup_enabled);
        assert!(saved.diagnostic_mode);

        let historical: i64 = repository
            .database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM app_configuration WHERE is_active=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(historical, 1);
    }

    #[test]
    fn stale_update_is_rejected_without_replacing_last_known_good() {
        let mut repository = ConfigurationRepository::open_in_memory().unwrap();
        let default = repository.ensure_default(10).unwrap();
        repository.update(&update_from(&default), 20).unwrap();

        let error = repository.update(&update_from(&default), 30).unwrap_err();
        assert!(matches!(error, ConfigurationRepositoryError::Conflict(_)));
        assert_eq!(repository.active().unwrap().unwrap().config_version, 2);
    }

    #[test]
    fn warmup_without_a_runtime_fails_closed() {
        let mut repository = ConfigurationRepository::open_in_memory().unwrap();
        let default = repository.ensure_default(10).unwrap();
        let mut update = update_from(&default);
        update.warmup_enabled = true;
        let error = repository.update(&update, 20).unwrap_err();
        assert!(matches!(error, ConfigurationRepositoryError::Invalid(_)));
        assert_eq!(repository.active().unwrap(), Some(default));
    }
}
