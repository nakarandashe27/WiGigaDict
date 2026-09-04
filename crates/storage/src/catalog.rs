use crate::model_manager::{
    ModelManagerError, ModelManagerResult, ModelManifest, SignedManifest, TrustedKeyRing,
};
use serde::{Deserialize, Serialize};

pub const CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_ENTRIES: usize = 64;

/// The shipped catalog of models a user may install. It is data, not code: the model screen
/// renders whatever this lists and knows nothing about individual models.
///
/// The document is signed as detached bytes (`catalog.json` + `catalog.sig`) so it stays
/// reviewable in a diff instead of collapsing into one escaped string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalog {
    pub schema_version: u32,
    pub catalog_version: u64,
    pub generated_at_ms: i64,
    pub signature_key_id: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub display_name: String,
    pub summary: String,
    pub languages: Vec<String>,
    pub requirements: CatalogRequirements,
    pub recommended: bool,
    /// True only where this project actually measured the model. The model screen must not show
    /// accuracy or speed numbers for anything that is false: we would be inventing them for a
    /// model we never ran.
    pub owner_measured: bool,
    /// Exact bytes of the package manifest, carried as text. The catalog signature covers them,
    /// and `ModelManager` verifies the manifest's own signature again before installing, so a
    /// tampered catalog cannot smuggle in a different package.
    pub manifest_json: String,
    pub manifest_signature_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRequirements {
    pub device_kind: String,
    pub min_ram_mb: u32,
    pub min_vram_mb: Option<u32>,
}

impl CatalogEntry {
    pub fn signed_manifest(&self) -> SignedManifest {
        SignedManifest {
            manifest_json: self.manifest_json.as_bytes().to_vec(),
            signature_hex: self.manifest_signature_hex.clone(),
        }
    }

    pub fn manifest(&self) -> ModelManagerResult<ModelManifest> {
        serde_json::from_str(&self.manifest_json)
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))
    }
}

/// Verifies a detached catalog signature and returns the parsed catalog.
///
/// An unsigned or unparseable catalog yields nothing at all: there is no "show it anyway" path,
/// because the entries are what the installer is asked to fetch.
pub fn verify_catalog(
    keys: &TrustedKeyRing,
    catalog_json: &[u8],
    signature_hex: &str,
) -> ModelManagerResult<ModelCatalog> {
    if catalog_json.is_empty() || catalog_json.len() > MAX_CATALOG_BYTES {
        return Err(ModelManagerError::InvalidManifest(
            "catalog size is outside 1..=1 MiB".into(),
        ));
    }
    let catalog: ModelCatalog = serde_json::from_slice(catalog_json)
        .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
    if catalog.schema_version != CATALOG_SCHEMA_VERSION {
        return Err(ModelManagerError::InvalidManifest(format!(
            "unsupported catalog schema version {}",
            catalog.schema_version
        )));
    }
    keys.verify_bytes(catalog_json, signature_hex, &catalog.signature_key_id)?;
    if catalog.entries.is_empty() || catalog.entries.len() > MAX_CATALOG_ENTRIES {
        return Err(ModelManagerError::InvalidManifest(
            "catalog must list between 1 and 64 entries".into(),
        ));
    }
    // A catalog the model screen cannot render is broken, not partially usable.
    for entry in &catalog.entries {
        entry.manifest()?;
    }
    Ok(catalog)
}
