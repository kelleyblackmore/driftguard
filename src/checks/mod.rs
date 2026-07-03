//! Check implementations and dispatch.

mod command;
mod config_file;
mod file;
mod package;
mod port;
mod process;
mod service;
#[cfg(test)]
mod tests;

use crate::config::CheckSpec;
use crate::runner::Runner;
use serde::Serialize;
use std::collections::BTreeMap;

/// Result of one executed check.
#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: String,
    #[serde(rename = "type")]
    pub check_type: String,
    pub passed: bool,
    pub message: String,
    pub details: BTreeMap<String, serde_json::Value>,
}

impl CheckResult {
    pub fn new(spec: &CheckSpec) -> Self {
        Self {
            name: spec.name.clone(),
            check_type: spec.check_type.clone(),
            passed: false,
            message: String::new(),
            details: BTreeMap::new(),
        }
    }

    pub fn pass(mut self, message: impl Into<String>) -> Self {
        self.passed = true;
        self.message = message.into();
        self
    }

    pub fn fail(mut self, message: impl Into<String>) -> Self {
        self.passed = false;
        self.message = message.into();
        self
    }

    pub fn detail(&mut self, key: &str, value: impl Into<serde_json::Value>) {
        self.details.insert(key.to_string(), value.into());
    }
}

/// Run a single check spec against the target.
pub fn run_check(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    match spec.check_type.as_str() {
        "file" | "directory" | "symlink" => file::run(runner, spec),
        "command" => command::run(runner, spec),
        "service" => service::run(runner, spec),
        "process" => process::run(runner, spec),
        "port" => port::run(runner, spec),
        "package" => package::run(runner, spec),
        "config" => config_file::run(runner, spec),
        other => CheckResult::new(spec).fail(format!("unknown check type: {other}")),
    }
}
