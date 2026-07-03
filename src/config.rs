//! Configuration loading, validation, and variable substitution.

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// A parsed driftguard configuration file.
///
/// Variable sections (`variables:` and the serverinspector-compatible
/// `environment.variables:`) are consumed during substitution before this
/// struct is deserialized, so they are not represented here.
#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "tests")]
    pub checks: Vec<CheckSpec>,
}

/// One check entry. `params` carries the type-specific keys.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub check_type: String,
    #[serde(flatten)]
    pub params: BTreeMap<String, serde_yaml::Value>,
}

impl CheckSpec {
    pub fn str_param(&self, key: &str) -> Option<String> {
        self.params.get(key).and_then(|v| match v {
            serde_yaml::Value::String(s) => Some(s.clone()),
            serde_yaml::Value::Number(n) => Some(n.to_string()),
            serde_yaml::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
    }

    pub fn bool_param(&self, key: &str) -> Option<bool> {
        self.params.get(key).and_then(serde_yaml::Value::as_bool)
    }

    pub fn int_param(&self, key: &str) -> Option<i64> {
        self.params.get(key).and_then(serde_yaml::Value::as_i64)
    }

    pub fn has_param(&self, key: &str) -> bool {
        self.params.contains_key(key)
    }
}

/// Load a configuration file, apply variable substitution, and validate it.
///
/// `overrides` are CLI-provided `--var KEY=VALUE` pairs; they take precedence
/// over variables defined in the file.
pub fn load(path: &Path, overrides: &BTreeMap<String, String>) -> Result<Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;

    let mut doc: serde_yaml::Value = serde_yaml::from_str(&raw)
        .with_context(|| format!("invalid YAML in {}", path.display()))?;

    let variables = collect_variables(&doc, overrides);
    if !variables.is_empty() {
        doc = substitute(doc, &variables);
    }

    let config: Config = serde_yaml::from_value(doc)
        .with_context(|| format!("invalid configuration structure in {}", path.display()))?;

    if config.checks.is_empty() {
        bail!("configuration must contain a non-empty 'checks' (or 'tests') list");
    }
    for (i, check) in config.checks.iter().enumerate() {
        if check.name.trim().is_empty() {
            bail!("check {} must have a non-empty 'name'", i + 1);
        }
    }

    Ok(config)
}

/// Gather variables from the document (`variables:` and the
/// serverinspector-compatible `environment.variables:`), then apply overrides.
fn collect_variables(
    doc: &serde_yaml::Value,
    overrides: &BTreeMap<String, String>,
) -> BTreeMap<String, serde_yaml::Value> {
    let mut vars: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();

    let mut absorb = |section: Option<&serde_yaml::Value>| {
        if let Some(serde_yaml::Value::Mapping(map)) = section {
            for (k, v) in map {
                if let serde_yaml::Value::String(name) = k {
                    vars.insert(name.clone(), v.clone());
                }
            }
        }
    };

    absorb(doc.get("variables"));
    absorb(doc.get("environment").and_then(|e| e.get("variables")));

    for (k, v) in overrides {
        vars.insert(k.clone(), serde_yaml::Value::String(v.clone()));
    }

    vars
}

/// Recursively substitute `{{ VAR }}` references in string values.
///
/// Substitution happens on the parsed structure, never on serialized text, so
/// values containing backslashes or YAML syntax cannot corrupt the document.
/// A string that is exactly one variable reference takes the variable's value
/// with its original type (numbers stay numbers, booleans stay booleans).
fn substitute(
    value: serde_yaml::Value,
    vars: &BTreeMap<String, serde_yaml::Value>,
) -> serde_yaml::Value {
    match value {
        serde_yaml::Value::String(s) => substitute_string(s, vars),
        serde_yaml::Value::Sequence(seq) => {
            serde_yaml::Value::Sequence(seq.into_iter().map(|v| substitute(v, vars)).collect())
        }
        serde_yaml::Value::Mapping(map) => serde_yaml::Value::Mapping(
            map.into_iter()
                .map(|(k, v)| (substitute(k, vars), substitute(v, vars)))
                .collect(),
        ),
        other => other,
    }
}

fn substitute_string(s: String, vars: &BTreeMap<String, serde_yaml::Value>) -> serde_yaml::Value {
    // Exact-match reference keeps the variable's native type.
    let exact = Regex::new(r"^\{\{\s*([A-Za-z0-9_]+)\s*\}\}$").unwrap();
    if let Some(caps) = exact.captures(&s) {
        if let Some(v) = vars.get(&caps[1]) {
            return v.clone();
        }
    }

    let embedded = Regex::new(r"\{\{\s*([A-Za-z0-9_]+)\s*\}\}").unwrap();
    let replaced = embedded.replace_all(&s, |caps: &regex::Captures| {
        match vars.get(&caps[1]) {
            Some(serde_yaml::Value::String(v)) => v.clone(),
            Some(serde_yaml::Value::Number(n)) => n.to_string(),
            Some(serde_yaml::Value::Bool(b)) => b.to_string(),
            // Unknown variable: leave the reference in place so the failure
            // is visible in check output instead of silently vanishing.
            _ => caps[0].to_string(),
        }
    });

    serde_yaml::Value::String(replaced.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn substitutes_windows_paths_safely() {
        let vars = BTreeMap::from([(
            "HOSTS".to_string(),
            serde_yaml::Value::String(r"C:\Windows\System32\drivers\etc\hosts".to_string()),
        )]);
        let doc = parse("path: '{{ HOSTS }}'");
        let out = substitute(doc, &vars);
        assert_eq!(
            out.get("path").unwrap().as_str().unwrap(),
            r"C:\Windows\System32\drivers\etc\hosts"
        );
    }

    #[test]
    fn exact_reference_keeps_type() {
        let vars = BTreeMap::from([("PORT".to_string(), serde_yaml::Value::Number(8080.into()))]);
        let doc = parse("port: '{{ PORT }}'");
        let out = substitute(doc, &vars);
        assert_eq!(out.get("port").unwrap().as_i64(), Some(8080));
    }

    #[test]
    fn embedded_reference_becomes_string() {
        let vars = BTreeMap::from([(
            "BASE".to_string(),
            serde_yaml::Value::String("/srv/app".to_string()),
        )]);
        let doc = parse("paths: ['{{ BASE }}/logs', '{{ BASE }}/data']");
        let out = substitute(doc, &vars);
        let seq = out.get("paths").unwrap().as_sequence().unwrap();
        assert_eq!(seq[0].as_str().unwrap(), "/srv/app/logs");
        assert_eq!(seq[1].as_str().unwrap(), "/srv/app/data");
    }

    #[test]
    fn unknown_variable_left_visible() {
        let vars = BTreeMap::new();
        let doc = parse("cmd: 'echo {{ MISSING }}'");
        let out = substitute(doc, &vars);
        assert_eq!(
            out.get("cmd").unwrap().as_str().unwrap(),
            "echo {{ MISSING }}"
        );
    }

    #[test]
    fn load_accepts_tests_alias_and_env_variables() {
        let dir = std::env::temp_dir().join("driftguard-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("compat.yaml");
        std::fs::write(
            &path,
            concat!(
                "title: compat\n",
                "environment:\n",
                "  variables:\n",
                "    MSG: hello\n",
                "tests:\n",
                "  - name: says hello\n",
                "    type: command\n",
                "    command: echo {{ MSG }}\n",
            ),
        )
        .unwrap();

        let config = load(&path, &BTreeMap::new()).unwrap();
        assert_eq!(config.checks.len(), 1);
        assert_eq!(config.checks[0].str_param("command").unwrap(), "echo hello");
    }

    #[test]
    fn cli_overrides_win() {
        let dir = std::env::temp_dir().join("driftguard-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("override.yaml");
        std::fs::write(
            &path,
            concat!(
                "variables:\n",
                "  TARGET: default\n",
                "checks:\n",
                "  - name: check target\n",
                "    type: command\n",
                "    command: echo {{ TARGET }}\n",
            ),
        )
        .unwrap();

        let overrides = BTreeMap::from([("TARGET".to_string(), "overridden".to_string())]);
        let config = load(&path, &overrides).unwrap();
        assert_eq!(
            config.checks[0].str_param("command").unwrap(),
            "echo overridden"
        );
    }
}
