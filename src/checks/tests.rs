//! Unit tests for check logic against a scripted mock runner.

use super::*;
use crate::config::CheckSpec;
use crate::runner::{CommandOutput, Runner};
use std::collections::HashMap;

/// Runner whose command responses are scripted per test: the first entry
/// whose needle is a substring of the executed command wins. Unmatched
/// commands return exit 127, like a missing binary.
#[derive(Default)]
struct MockRunner {
    responses: Vec<(String, CommandOutput)>,
    files: HashMap<String, String>,
    local: bool,
}

impl MockRunner {
    fn on(mut self, needle: &str, exit_code: i32, stdout: &str, stderr: &str) -> Self {
        self.responses.push((
            needle.to_string(),
            CommandOutput {
                exit_code,
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            },
        ));
        self
    }

    fn with_file(mut self, path: &str, content: &str) -> Self {
        self.files.insert(path.to_string(), content.to_string());
        self
    }
}

impl Runner for MockRunner {
    fn target(&self) -> String {
        "mock".to_string()
    }

    fn is_local(&self) -> bool {
        self.local
    }

    fn run(&self, command: &str) -> anyhow::Result<CommandOutput> {
        for (needle, output) in &self.responses {
            if command.contains(needle.as_str()) {
                return Ok(output.clone());
            }
        }
        Ok(CommandOutput {
            exit_code: 127,
            stdout: String::new(),
            stderr: format!("mock: no response scripted for: {command}"),
        })
    }

    fn file_exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    fn read_file(&self, path: &str) -> anyhow::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("mock: no such file {path}"))
    }
}

fn spec(yaml: &str) -> CheckSpec {
    serde_yaml::from_str(yaml).unwrap()
}

// ---- dispatch ----

#[test]
fn unknown_check_type_fails_cleanly() {
    let result = run_check(&MockRunner::default(), &spec("{name: x, type: quantum}"));
    assert!(!result.passed);
    assert!(result.message.contains("unknown check type"));
}

// ---- command ----

#[test]
fn command_pass_with_exit_and_stdout_assertions() {
    let runner = MockRunner::default().on("nginx -t", 0, "syntax is ok", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: command, command: nginx -t, exit_status: 0, stdout: {contains: syntax is ok}}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn command_fails_on_wrong_exit_code() {
    let runner = MockRunner::default().on("false", 1, "", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: command, command: false, exit_status: 0}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("exited with 1"));
}

#[test]
fn command_fails_on_missing_stdout_content() {
    let runner = MockRunner::default().on("echo", 0, "goodbye", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: command, command: echo hi, stdout_contains: hello}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("does not contain"));
}

#[test]
fn command_checks_stderr_pattern() {
    let runner = MockRunner::default().on("nginx -v", 0, "", "nginx version: nginx/1.24.0");
    let result = run_check(
        &runner,
        &spec(r"{name: x, type: command, command: nginx -v, stderr: {pattern: 'nginx/\d+\.\d+'}}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn command_requires_command_param() {
    let result = run_check(&MockRunner::default(), &spec("{name: x, type: command}"));
    assert!(!result.passed);
    assert!(result.message.contains("missing required parameter"));
}

// ---- file ----

#[test]
fn file_exists_and_contains() {
    let runner = MockRunner::default().with_file("/etc/hosts", "127.0.0.1 localhost");
    let result = run_check(
        &runner,
        &spec("{name: x, type: file, path: /etc/hosts, exists: true, contains: localhost}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn file_missing_fails() {
    let result = run_check(
        &MockRunner::default(),
        &spec("{name: x, type: file, path: /nope, exists: true}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("does not exist"));
}

#[test]
fn file_absence_can_be_expected() {
    let result = run_check(
        &MockRunner::default(),
        &spec("{name: x, type: file, path: /nope, exists: false}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn file_content_pattern_mismatch_fails() {
    let runner = MockRunner::default().with_file("/etc/app.conf", "mode = debug");
    let result = run_check(
        &runner,
        &spec("{name: x, type: file, path: /etc/app.conf, content_pattern: 'mode = production'}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("does not match"));
}

#[test]
fn file_permissions_checked_via_stat() {
    let runner = MockRunner::default()
        .with_file("/etc/shadow", "")
        .on("stat", 0, "640\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: file, path: /etc/shadow, permissions: '640'}"),
    );
    assert!(result.passed, "{}", result.message);

    let runner = MockRunner::default()
        .with_file("/etc/shadow", "")
        .on("stat", 0, "777\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: file, path: /etc/shadow, permissions: '640'}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("has permissions 777"));
}

#[test]
fn directory_type_verified_remotely() {
    let runner = MockRunner::default()
        .with_file("/var/www", "")
        .on("test -d", 0, "", "");
    let result = run_check(&runner, &spec("{name: x, type: directory, path: /var/www}"));
    assert!(result.passed, "{}", result.message);
}

// ---- service ----

#[test]
fn service_running_via_systemctl() {
    let runner = MockRunner::default().on("systemctl is-active", 0, "active\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: service, service: nginx, running: true}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn service_inactive_fails_running_check() {
    let runner = MockRunner::default()
        .on("systemctl is-active", 3, "inactive\n", "")
        .on("service", 3, "", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: service, service: nginx, running: true}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("not running"));
}

#[test]
fn service_absence_can_be_expected() {
    let runner = MockRunner::default()
        .on("systemctl is-active", 3, "inactive\n", "")
        .on("service", 3, "", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: service, service: debug-agent, running: false}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn service_enabled_check() {
    let runner = MockRunner::default()
        .on("systemctl is-active", 0, "active\n", "")
        .on("systemctl is-enabled", 0, "disabled\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: service, service: nginx, running: true, enabled: true}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("disabled"));
}

// ---- process ----

#[test]
fn process_running_via_pgrep() {
    let runner = MockRunner::default().on("pgrep", 0, "3\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: process, process: nginx, running: true, min_count: 2}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn process_count_bounds_enforced() {
    let runner = MockRunner::default().on("pgrep", 0, "5\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: process, process: nginx, max_count: 2}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("at most 2"));

    let runner = MockRunner::default().on("pgrep", 0, "1\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: process, process: nginx, count: 2}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("exactly 2"));
}

#[test]
fn process_absent_via_ps_fallback() {
    // pgrep missing (exit 127) -> falls back to parsing ps output.
    let runner = MockRunner::default().on("ps -e", 0, "systemd\nsshd\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: process, process: nginx, running: false}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn process_found_in_ps_fallback() {
    let runner = MockRunner::default().on("ps -e", 0, "systemd\n/usr/sbin/nginx\nnginx\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: process, process: nginx, running: true}"),
    );
    assert!(result.passed, "{}", result.message);
}

// ---- port (remote socket-table path) ----

#[test]
fn port_listening_in_ss_table() {
    let runner = MockRunner::default().on("ss -tln", 0, "LISTEN 0 511 0.0.0.0:80 0.0.0.0:*\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: port, port: 80, listening: true}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn port_not_listening_fails() {
    let runner = MockRunner::default().on("ss -tln", 0, "LISTEN 0 128 0.0.0.0:22 0.0.0.0:*\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: port, port: 8080, listening: true}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("not listening"));
}

#[test]
fn port_rejects_invalid_number() {
    let result = run_check(
        &MockRunner::default(),
        &spec("{name: x, type: port, port: 99999}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("invalid"));
}

#[test]
fn port_udp_uses_udp_table() {
    let runner = MockRunner::default().on("ss -uln", 0, "UNCONN 0 0 0.0.0.0:53 0.0.0.0:*\n", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: port, port: 53, protocol: udp}"),
    );
    assert!(result.passed, "{}", result.message);
}

// ---- package ----

#[test]
fn package_installed_via_dpkg() {
    let runner = MockRunner::default()
        .on("command -v dpkg-query", 0, "", "")
        .on("dpkg-query", 0, "1.24.0-1ubuntu1", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: package, package: nginx, installed: true}"),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn package_missing_fails_installed_check() {
    let runner = MockRunner::default()
        .on("command -v dpkg-query", 0, "", "")
        .on("dpkg-query", 1, "", "no packages found");
    let result = run_check(
        &runner,
        &spec("{name: x, type: package, package: nginx, installed: true}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("not installed"));
}

#[test]
fn package_version_prefix_match() {
    let runner = MockRunner::default()
        .on("command -v dpkg-query", 0, "", "")
        .on("dpkg-query", 0, "1.24.0-1ubuntu1", "");
    let result = run_check(
        &runner,
        &spec("{name: x, type: package, package: nginx, version: '1.24'}"),
    );
    assert!(result.passed, "{}", result.message);

    let result = run_check(
        &runner,
        &spec("{name: x, type: package, package: nginx, version: '1.99'}"),
    );
    assert!(!result.passed);
}

#[test]
fn package_reports_no_manager_found() {
    // All availability probes return 127 (nothing scripted).
    let result = run_check(
        &MockRunner::default(),
        &spec("{name: x, type: package, package: nginx}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("no supported package manager"));
}

// ---- config ----

#[test]
fn config_yaml_key_and_value_assertions() {
    let runner = MockRunner::default().with_file(
        "/etc/app/settings.yaml",
        "environment: production\nlogging:\n  level: info\n",
    );
    let result = run_check(
        &runner,
        &spec(concat!(
            "{name: x, type: config, path: /etc/app/settings.yaml, format: yaml, ",
            "has_key: logging.level, has_value: {environment: production, logging.level: info}}"
        )),
    );
    assert!(result.passed, "{}", result.message);
}

#[test]
fn config_missing_key_fails() {
    let runner = MockRunner::default().with_file("/etc/app.json", r#"{"a": 1}"#);
    let result = run_check(
        &runner,
        &spec("{name: x, type: config, path: /etc/app.json, has_key: b}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("missing key"));
}

#[test]
fn config_wrong_value_fails() {
    let runner = MockRunner::default().with_file("/etc/app.json", r#"{"port": 9090}"#);
    let result = run_check(
        &runner,
        &spec("{name: x, type: config, path: /etc/app.json, has_value: {port: 8080}}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("expected 8080"));
}

#[test]
fn config_invalid_json_fails() {
    let runner = MockRunner::default().with_file("/etc/app.json", "not json {");
    let result = run_check(
        &runner,
        &spec("{name: x, type: config, path: /etc/app.json, has_key: a}"),
    );
    assert!(!result.passed);
    assert!(result.message.contains("not valid JSON"));
}
