//! File, directory, and symlink checks: existence, type, content, permissions.

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::{shell_quote, Runner};
use regex::Regex;

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(path) = spec.str_param("path") else {
        return result.fail("missing required parameter: path");
    };
    result.detail("path", path.clone());

    let exists = runner.file_exists(&path);
    result.detail("exists", exists);

    let expect_exists = spec.bool_param("exists").unwrap_or(true);
    if exists != expect_exists {
        return result.fail(if expect_exists {
            format!("{path} does not exist")
        } else {
            format!("{path} exists but should not")
        });
    }
    if !expect_exists {
        return result.pass(format!("{path} correctly absent"));
    }

    // Type check: the check_type itself may narrow the expectation
    // (type: directory / type: symlink), mirroring serverinspector configs.
    if spec.check_type == "directory" || spec.check_type == "symlink" {
        let flag = if spec.check_type == "directory" {
            "-d"
        } else {
            "-L"
        };
        let ok = if runner.is_local() && spec.check_type == "directory" {
            std::path::Path::new(&path).is_dir()
        } else if runner.is_local() && spec.check_type == "symlink" {
            std::path::Path::new(&path).is_symlink()
        } else {
            runner
                .run(&format!("test {flag} {}", shell_quote(&path)))
                .map(|o| o.exit_code == 0)
                .unwrap_or(false)
        };
        if !ok {
            return result.fail(format!("{path} is not a {}", spec.check_type));
        }
    }

    // Content checks read the file once and evaluate all assertions on it.
    let wants_content = spec.has_param("content")
        || spec.has_param("contains")
        || spec.has_param("content_pattern");
    if wants_content {
        let content = match runner.read_file(&path) {
            Ok(c) => c,
            Err(e) => return result.fail(format!("could not read {path}: {e}")),
        };

        for key in ["content", "contains"] {
            if let Some(needle) = spec.str_param(key) {
                result.detail("expected_content", needle.clone());
                if !content.contains(&needle) {
                    return result.fail(format!("{path} does not contain: {needle}"));
                }
            }
        }

        if let Some(pattern) = spec.str_param("content_pattern") {
            result.detail("content_pattern", pattern.clone());
            match Regex::new(&pattern) {
                Ok(re) => {
                    if !re.is_match(&content) {
                        return result.fail(format!("{path} does not match pattern: {pattern}"));
                    }
                }
                Err(e) => return result.fail(format!("invalid content_pattern: {e}")),
            }
        }
    }

    // Permission check (octal string like "644"); Unix targets only.
    if let Some(expected_perms) = spec.str_param("permissions") {
        result.detail("expected_permissions", expected_perms.clone());
        let out = runner.run(&format!(
            "stat -c %a {} 2>/dev/null || stat -f %Lp {}",
            shell_quote(&path),
            shell_quote(&path)
        ));
        match out {
            Ok(o) if o.exit_code == 0 => {
                let actual = o.stdout.trim().to_string();
                result.detail("permissions", actual.clone());
                if actual != expected_perms {
                    return result.fail(format!(
                        "{path} has permissions {actual}, expected {expected_perms}"
                    ));
                }
            }
            _ => return result.fail(format!("could not stat {path} for permissions")),
        }
    }

    result.pass(format!("{path} passed all checks"))
}
