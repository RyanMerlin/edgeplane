use mc::schema_pack::SchemaPack;
use serde_json::json;
use std::{env, fs};
use tempfile::NamedTempFile;

#[test]
fn default_pack_validates_known_entity() {
    let pack = SchemaPack::default();
    let payload = json!({
        "entity_type": "mission",
        "name": "launch",
        "description": "example mission"
    });
    assert!(pack.validate_payload(&payload).is_ok());
}

#[test]
fn default_pack_rejects_missing_fields() {
    let pack = SchemaPack::default();
    let payload = json!({
        "entity_type": "kluster",
        "description": "missing name"
    });
    let err = pack.validate_payload(&payload).unwrap_err();
    assert!(
        err.to_string()
            .contains("missing required fields [\"name\"] for entity 'kluster'")
    );
}

#[test]
fn load_prefers_env_schema_pack() {
    let temp = NamedTempFile::new().expect("create temp file");
    let content = r#"{
        "version": "v2",
        "name": "override",
        "entities": {
            "custom": {
                "required": ["name"],
                "optional": []
            }
        }
    }"#;
    fs::write(temp.path(), content).expect("write schema pack");
    // Rust 2024 marks environment mutation as unsafe because it is process-global.
    unsafe { env::set_var("MC_SCHEMA_PACK_FILE", temp.path()) };
    let pack = SchemaPack::load();
    unsafe { env::remove_var("MC_SCHEMA_PACK_FILE") };
    assert_eq!(pack.version, "v2");
    assert_eq!(pack.name, "override");
    assert!(pack.entities.contains_key("custom"));
}
