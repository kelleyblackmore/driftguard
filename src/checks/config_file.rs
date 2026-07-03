//! Structured config-file checks: assert keys and values in JSON or YAML
//! files on the target (e.g. an app config that Ansible templated out).

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::Runner;

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(path) = spec.str_param("path") else {
        return result.fail("missing required parameter: path");
    };
    result.detail("path", path.clone());

    if !runner.file_exists(&path) {
        return result.fail(format!("{path} does not exist"));
    }

    let raw = match runner.read_file(&path) {
        Ok(c) => c,
        Err(e) => return result.fail(format!("could not read {path}: {e}")),
    };

    let format = spec
        .str_param("format")
        .unwrap_or_else(|| guess_format(&path))
        .to_lowercase();
    result.detail("format", format.clone());

    // Both JSON and YAML parse into serde_json::Value for uniform lookup.
    let doc: serde_json::Value = match format.as_str() {
        "json" => match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return result.fail(format!("{path} is not valid JSON: {e}")),
        },
        "yaml" | "yml" => match serde_yaml::from_str::<serde_yaml::Value>(&raw) {
            Ok(y) => match serde_json::to_value(&y) {
                Ok(v) => v,
                Err(e) => return result.fail(format!("could not convert YAML: {e}")),
            },
            Err(e) => return result.fail(format!("{path} is not valid YAML: {e}")),
        },
        other => return result.fail(format!("unsupported format: {other} (json or yaml)")),
    };

    // has_key: "a.b.c" or a list of such paths.
    if let Some(key_param) = spec.params.get("has_key") {
        let keys = string_or_list(key_param);
        for key in keys {
            if lookup(&doc, &key).is_none() {
                return result.fail(format!("{path} is missing key: {key}"));
            }
            result.detail(&format!("has_key.{key}"), true);
        }
    }

    // has_value: mapping of dotted key path -> expected value.
    if let Some(serde_yaml::Value::Mapping(map)) = spec.params.get("has_value") {
        for (k, expected_yaml) in map {
            let Some(key) = k.as_str() else { continue };
            let Some(actual) = lookup(&doc, key) else {
                return result.fail(format!("{path} is missing key: {key}"));
            };
            let expected: serde_json::Value = match serde_json::to_value(expected_yaml) {
                Ok(v) => v,
                Err(e) => return result.fail(format!("bad expected value for {key}: {e}")),
            };
            if !values_match(actual, &expected) {
                return result.fail(format!(
                    "{path}: key {key} is {actual}, expected {expected}"
                ));
            }
            result.detail(&format!("value.{key}"), actual.clone());
        }
    }

    result.pass(format!("{path} passed all checks"))
}

fn guess_format(path: &str) -> String {
    let lower = path.to_lowercase();
    if lower.ends_with(".json") {
        "json".to_string()
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "yaml".to_string()
    } else {
        "json".to_string()
    }
}

fn string_or_list(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::String(s) => vec![s.clone()],
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
}

/// Dotted-path lookup: "service.port" -> doc["service"]["port"].
fn lookup<'a>(doc: &'a serde_json::Value, dotted: &str) -> Option<&'a serde_json::Value> {
    let mut current = doc;
    for part in dotted.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(part)?,
            serde_json::Value::Array(arr) => arr.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Loose equality: numbers compare numerically, everything else strictly,
/// with a string-representation fallback so `port: "8080"` matches 8080.
fn values_match(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    if actual == expected {
        return true;
    }
    match (actual.as_f64(), expected.as_f64()) {
        (Some(a), Some(e)) => a == e,
        _ => value_to_string(actual) == value_to_string(expected),
    }
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_walks_nested_objects_and_arrays() {
        let doc: serde_json::Value =
            serde_json::json!({"service": {"ports": [80, 443], "enabled": true}});
        assert_eq!(
            lookup(&doc, "service.enabled"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            lookup(&doc, "service.ports.1"),
            Some(&serde_json::json!(443))
        );
        assert_eq!(lookup(&doc, "service.missing"), None);
    }

    #[test]
    fn values_match_is_type_tolerant() {
        assert!(values_match(
            &serde_json::json!(8080),
            &serde_json::json!(8080.0)
        ));
        assert!(values_match(
            &serde_json::json!("8080"),
            &serde_json::json!(8080)
        ));
        assert!(!values_match(
            &serde_json::json!("8081"),
            &serde_json::json!(8080)
        ));
    }
}
