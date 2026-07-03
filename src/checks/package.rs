//! Package checks: is a package installed (dpkg, rpm, apk, pacman).

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::{shell_quote, Runner};

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(package) = spec.str_param("package") else {
        return result.fail("missing required parameter: package");
    };
    result.detail("package", package.clone());

    if runner.is_local() && cfg!(windows) {
        return result.fail("package checks are not supported on local Windows targets");
    }

    let expect_installed = spec.bool_param("installed").unwrap_or(true);

    let (installed, manager, version) = query_package(runner, &package);
    result.detail("installed", installed);
    if let Some(m) = &manager {
        result.detail("package_manager", m.clone());
    }
    if let Some(v) = &version {
        result.detail("installed_version", v.clone());
    }

    if manager.is_none() {
        return result.fail("no supported package manager found on target (dpkg/rpm/apk/pacman)");
    }

    if installed != expect_installed {
        return result.fail(format!(
            "package {package} is {}, expected {}",
            if installed {
                "installed"
            } else {
                "not installed"
            },
            if expect_installed {
                "installed"
            } else {
                "not installed"
            }
        ));
    }

    if let Some(expected_version) = spec.str_param("version") {
        result.detail("expected_version", expected_version.clone());
        match &version {
            Some(v) if v.starts_with(&expected_version) => {}
            Some(v) => {
                return result.fail(format!(
                    "package {package} version {v} does not match expected {expected_version}"
                ))
            }
            None => {
                return result.fail(format!(
                    "could not determine installed version of {package}"
                ))
            }
        }
    }

    result.pass(format!("package {package} passed all checks"))
}

/// Probe package managers in order. Returns (installed, manager, version).
fn query_package(runner: &dyn Runner, package: &str) -> (bool, Option<String>, Option<String>) {
    let quoted = shell_quote(package);

    let probes: [(&str, String, String); 4] = [
        (
            "dpkg",
            format!("dpkg-query -W -f '${{Version}}' {quoted} 2>/dev/null"),
            "command -v dpkg-query >/dev/null 2>&1".to_string(),
        ),
        (
            "rpm",
            format!("rpm -q --qf '%{{VERSION}}-%{{RELEASE}}' {quoted} 2>/dev/null"),
            "command -v rpm >/dev/null 2>&1".to_string(),
        ),
        (
            "apk",
            format!(
                "apk info -e {quoted} 2>/dev/null && apk version {quoted} 2>/dev/null | tail -1"
            ),
            "command -v apk >/dev/null 2>&1".to_string(),
        ),
        (
            "pacman",
            format!("pacman -Q {quoted} 2>/dev/null"),
            "command -v pacman >/dev/null 2>&1".to_string(),
        ),
    ];

    for (manager, query, availability) in probes {
        let available = runner
            .run(&availability)
            .map(|o| o.exit_code == 0)
            .unwrap_or(false);
        if !available {
            continue;
        }
        let out = runner.run(&query);
        return match out {
            Ok(o) if o.exit_code == 0 => {
                let version = o
                    .stdout
                    .split_whitespace()
                    .last()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                (true, Some(manager.to_string()), version)
            }
            _ => (false, Some(manager.to_string()), None),
        };
    }

    (false, None, None)
}
