use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};
use thiserror::Error;
use tracing::warn;

/// Represents the subset of Mission Control entities that must satisfy schema packs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SchemaPack {
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entities: HashMap<String, EntitySpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EntitySpec {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

#[derive(Clone, Debug, Error)]
#[error("missing required fields {fields:?} for entity '{entity}'")]
pub struct SchemaValidationError {
    entity: String,
    fields: Vec<String>,
}

impl SchemaPack {
    pub fn load() -> Self {
        let maybe_path = env::var("MC_SCHEMA_PACK_FILE")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(default_path);

        let path = match maybe_path {
            Some(path) => path,
            None => return Self::default(),
        };

        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<SchemaPack>(&contents) {
                Ok(pack) if !pack.entities.is_empty() => pack,
                _ => {
                    warn!(path = ?path, "schema pack invalid; falling back to defaults");
                    Self::default()
                }
            },
            Err(err) => {
                warn!(path = ?path, error = %err, "unable to read schema pack; using default");
                Self::default()
            }
        }
    }

    pub fn validate_payload(&self, payload: &Value) -> Result<(), SchemaValidationError> {
        let entity_type = payload
            .get("entity_type")
            .or_else(|| payload.get("type"))
            .and_then(Value::as_str)
            .map(|value| value.to_lowercase());

        let payload_map = match payload.as_object() {
            Some(map) => map,
            None => return Ok(()),
        };

        if let Some(entity_key) = entity_type {
            if let Some(spec) = self.entities.get(&entity_key) {
                let missing: Vec<String> = spec
                    .required
                    .iter()
                    .filter(|field| payload_map.get(*field).map(is_missing).unwrap_or(true))
                    .cloned()
                    .collect();
                if missing.is_empty() {
                    return Ok(());
                }
                return Err(SchemaValidationError {
                    entity: entity_key,
                    fields: missing,
                });
            }
        }
        Ok(())
    }
}

impl Default for SchemaPack {
    fn default() -> Self {
        let mut entities = HashMap::new();
        entities.insert(
            "mission".into(),
            EntitySpec {
                required: vec!["name".into()],
                optional: vec![
                    "description".into(),
                    "owners".into(),
                    "contributors".into(),
                    "tags".into(),
                    "visibility".into(),
                    "status".into(),
                ],
            },
        );
        entities.insert(
            "kluster".into(),
            EntitySpec {
                required: vec!["name".into()],
                optional: vec![
                    "description".into(),
                    "owners".into(),
                    "contributors".into(),
                    "tags".into(),
                    "status".into(),
                    "mission_id".into(),
                ],
            },
        );
        entities.insert(
            "task".into(),
            EntitySpec {
                required: vec!["kluster_id".into(), "title".into()],
                optional: vec![
                    "description".into(),
                    "status".into(),
                    "owner".into(),
                    "contributors".into(),
                    "dependencies".into(),
                    "definition_of_done".into(),
                    "related_artifacts".into(),
                ],
            },
        );
        entities.insert(
            "doc".into(),
            EntitySpec {
                required: vec!["kluster_id".into(), "title".into(), "body".into()],
                optional: vec!["doc_type".into(), "status".into(), "provenance".into()],
            },
        );
        entities.insert(
            "artifact".into(),
            EntitySpec {
                required: vec!["kluster_id".into(), "name".into(), "uri".into()],
                optional: vec!["artifact_type".into(), "status".into(), "provenance".into()],
            },
        );
        Self {
            version: "v1".into(),
            name: "main".into(),
            description: "Default mission/kluster/task schema".into(),
            entities,
        }
    }
}

fn default_path() -> Option<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|parent| parent.parent())
        .map(|root| root.join("docs").join("schema-packs").join("main.json"))
}

fn is_missing(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}
