use std::collections::BTreeMap;

use tracing::{error, info};

pub(crate) fn persist(
    data_root: &std::path::Path,
    changes: &BTreeMap<String, serde_json::Value>,
) -> bool {
    if changes
        .keys()
        .any(|key| !crate::config::manager_fields::is_known_key(key))
    {
        error!("config_persistence.unknown_field_rejected");
        return false;
    }
    let dir = data_root;
    if let Err(e) = std::fs::create_dir_all(dir) {
        error!(error = %e, "config_persistence.mkdir_failed");
        return false;
    }

    let config_path = dir.join("config.toml");

    let existing = match std::fs::read_to_string(&config_path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            error!(%error, "config_persistence.read_failed");
            return false;
        }
    };

    let mut doc: toml_edit::DocumentMut = match existing.parse() {
        Ok(d) => d,
        Err(e) => {
            error!(error = %e, "config_persistence.parse_failed");
            return false;
        }
    };

    for (dotted_key, value) in changes {
        if value.is_null() {
            remove_toml_value(&mut doc, dotted_key);
        } else if !set_toml_value(&mut doc, dotted_key, value) {
            return false;
        }
    }

    let content = render_config(&existing, &doc);

    if let Err(e) = crate::util::durable_fs::atomic_write_sync(&config_path, content.as_bytes()) {
        error!(error = %e, "config_persistence.write_failed");
        return false;
    }

    info!(keys = ?changes.keys().collect::<Vec<_>>(), "config_persistence.persisted");
    true
}

pub(crate) fn remove(config_root: &std::path::Path, dotted_key: &str) -> bool {
    if !crate::config::manager_fields::is_known_key(dotted_key) {
        error!(
            key = dotted_key,
            "config_persistence.unknown_field_rejected"
        );
        return false;
    }
    let config_path = config_root.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(error) => {
            error!(%error, "config_persistence.read_failed");
            return false;
        }
    };
    let mut doc: toml_edit::DocumentMut = match existing.parse() {
        Ok(document) => document,
        Err(error) => {
            error!(%error, "config_persistence.parse_failed");
            return false;
        }
    };
    let removed = remove_toml_value_inner(&mut doc, dotted_key);
    if !removed {
        return true;
    }
    if let Err(error) =
        crate::util::durable_fs::atomic_write_sync(&config_path, doc.to_string().as_bytes())
    {
        error!(%error, "config_persistence.write_failed");
        return false;
    }
    info!(key = dotted_key, "config_persistence.removed");
    true
}

fn remove_toml_value(doc: &mut toml_edit::DocumentMut, dotted_key: &str) {
    let _ = remove_toml_value_inner(doc, dotted_key);
}

fn remove_toml_value_inner(doc: &mut toml_edit::DocumentMut, dotted_key: &str) -> bool {
    let parts: Vec<&str> = dotted_key.split('.').collect();
    let Some((field, sections)) = parts.split_last() else {
        return false;
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for section in sections {
        let Some(next) = table
            .get_mut(section)
            .and_then(toml_edit::Item::as_table_like_mut)
        else {
            return false;
        };
        table = next;
    }
    table.remove(field).is_some()
}

fn set_toml_value(
    doc: &mut toml_edit::DocumentMut,
    dotted_key: &str,
    value: &serde_json::Value,
) -> bool {
    let parts: Vec<&str> = dotted_key.split('.').collect();
    let Some((field, sections)) = parts.split_last() else {
        return false;
    };
    let Some(item) = json_to_toml_item(value) else {
        error!(
            key = dotted_key,
            "config_persistence.value_not_representable"
        );
        return false;
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for section in sections {
        let Some(next) = table
            .entry(section)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
        else {
            error!(
                key = dotted_key,
                section, "config_persistence.path_is_not_table"
            );
            return false;
        };
        table = next;
    }
    table.insert(field, item);
    true
}

pub(crate) fn render_config(existing: &str, document: &toml_edit::DocumentMut) -> String {
    if existing.trim().is_empty() {
        format!("# BoBe configuration\n{document}")
    } else {
        document.to_string()
    }
}

fn json_to_toml_item(value: &serde_json::Value) -> Option<toml_edit::Item> {
    use serde::Serialize;
    match value {
        serde_json::Value::Null => Some(toml_edit::Item::None),
        serde_json::Value::Object(map) => {
            let mut table = toml_edit::Table::new();
            for (key, value) in map.iter().filter(|(_, value)| !value.is_null()) {
                table.insert(key, json_to_toml_item(value)?);
            }
            Some(toml_edit::Item::Table(table))
        }
        value => value
            .serialize(toml_edit::ser::ValueSerializer::new())
            .ok()
            .map(toml_edit::Item::Value),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::persist;

    #[test]
    fn removal_deletes_only_the_requested_persisted_key() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-remove-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test config directory");
        std::fs::write(
            root.join("config.toml"),
            "[voice]\nenabled = false\npersona = 'marius'\n",
        )
        .expect("write test config");

        assert!(super::remove(&root, "voice.enabled"));
        let saved = std::fs::read_to_string(root.join("config.toml")).expect("read saved config");
        assert!(!saved.contains("enabled"));
        assert!(saved.contains("persona"));
        assert!(std::fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn inline_tables_are_edited_in_place() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-inline-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test config directory");
        std::fs::write(
            root.join("config.toml"),
            "voice = { enabled = false, persona = 'marius' }\n",
        )
        .expect("write test config");

        let changes = BTreeMap::from([("voice.enabled".into(), serde_json::Value::Bool(true))]);
        assert!(persist(&root, &changes));
        let saved = std::fs::read_to_string(root.join("config.toml")).expect("read saved config");
        assert!(saved.contains("enabled = true"), "{saved}");

        assert!(super::remove(&root, "voice.enabled"));
        let saved = std::fs::read_to_string(root.join("config.toml")).expect("read saved config");
        assert!(!saved.contains("enabled"), "{saved}");
        assert!(saved.contains("persona"), "{saved}");
        assert!(std::fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn null_change_removes_optional_path_instead_of_writing_empty_string() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-null-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test config directory");
        std::fs::write(
            root.join("config.toml"),
            "[server]\ntls_cert_path = '/tmp/cert.pem'\ntls_key_path = '/tmp/key.pem'\n",
        )
        .expect("write test config");
        let changes = BTreeMap::from([
            ("server.tls_cert_path".into(), serde_json::Value::Null),
            ("server.tls_key_path".into(), serde_json::Value::Null),
        ]);

        assert!(persist(&root, &changes));
        let saved = std::fs::read_to_string(root.join("config.toml")).expect("read saved config");
        assert!(!saved.contains("tls_cert_path"));
        assert!(!saved.contains("tls_key_path"));
        assert!(std::fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn first_write_starts_with_the_header() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-header-test-{}", uuid::Uuid::new_v4()));
        let changes = BTreeMap::from([("capture.enabled".to_owned(), serde_json::json!(false))]);

        assert!(persist(&root, &changes));
        let saved = std::fs::read_to_string(root.join("config.toml")).expect("read saved config");
        assert_eq!(saved, "# BoBe configuration\n[capture]\nenabled = false\n");
        assert!(std::fs::remove_dir_all(root).is_ok());
    }

    #[test]
    fn object_value_persists_as_a_table_that_loads_back() {
        let root =
            std::env::temp_dir().join(format!("bobe-config-object-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test config directory");
        std::fs::write(root.join("config.toml"), "[agent]\nenabled_tools = []\n")
            .expect("write test config");
        let execution = crate::config::ExecutionLimits {
            model_concurrency: 6,
            ..crate::config::ExecutionLimits::default()
        };
        let changes = BTreeMap::from([(
            "agent.execution".to_owned(),
            serde_json::to_value(&execution).expect("serialize execution limits"),
        )]);

        assert!(persist(&root, &changes));
        let loaded = crate::config::Config::load_from(&root.join("config.toml"), &root)
            .expect("reload saved config");
        assert_eq!(loaded.config.agent.execution, execution);
        assert!(loaded.config.agent.enabled_tools.is_empty());
        assert!(std::fs::remove_dir_all(root).is_ok());
    }
}
