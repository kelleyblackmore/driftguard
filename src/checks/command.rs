//! Command execution checks: exit status, stdout/stderr content and patterns.

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::Runner;
use regex::Regex;

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(command) = spec.str_param("command") else {
        return result.fail("missing required parameter: command");
    };
    result.detail("command", command.clone());

    let out = match runner.run(&command) {
        Ok(o) => o,
        Err(e) => return result.fail(format!("failed to execute command: {e}")),
    };
    result.detail("exit_code", out.exit_code);
    result.detail("stdout", out.stdout.clone());
    result.detail("stderr", out.stderr.clone());

    // exit_status (serverinspector compat) and exit_code both accepted.
    let expected_exit = spec
        .int_param("exit_status")
        .or_else(|| spec.int_param("exit_code"));
    if let Some(expected) = expected_exit {
        result.detail("expected_exit_code", expected);
        if i64::from(out.exit_code) != expected {
            return result.fail(format!(
                "command exited with {}, expected {expected}",
                out.exit_code
            ));
        }
    }

    // stdout / stderr assertions, either flat (stdout_contains) or nested
    // (stdout: {contains: ..., pattern: ...}).
    for stream in ["stdout", "stderr"] {
        let text = if stream == "stdout" {
            &out.stdout
        } else {
            &out.stderr
        };

        let mut contains: Option<String> = spec.str_param(&format!("{stream}_contains"));
        let mut pattern: Option<String> = spec.str_param(&format!("{stream}_pattern"));

        if let Some(serde_yaml::Value::Mapping(map)) = spec.params.get(stream) {
            if let Some(v) = map.get("contains").and_then(|v| v.as_str()) {
                contains = Some(v.to_string());
            }
            if let Some(v) = map.get("pattern").and_then(|v| v.as_str()) {
                pattern = Some(v.to_string());
            }
        }

        if let Some(needle) = contains {
            result.detail(&format!("expected_{stream}_contains"), needle.clone());
            if !text.contains(&needle) {
                return result.fail(format!("{stream} does not contain: {needle}"));
            }
        }
        if let Some(pat) = pattern {
            result.detail(&format!("{stream}_pattern"), pat.clone());
            match Regex::new(&pat) {
                Ok(re) => {
                    if !re.is_match(text) {
                        return result.fail(format!("{stream} does not match pattern: {pat}"));
                    }
                }
                Err(e) => return result.fail(format!("invalid {stream} pattern: {e}")),
            }
        }
    }

    result.pass("command passed all checks")
}
