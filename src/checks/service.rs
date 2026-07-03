//! Service checks: running / enabled state via systemctl, with SysV fallback
//! and a Windows branch for local targets.

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::{shell_quote, Runner};

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(service) = spec
        .str_param("service")
        .or_else(|| spec.str_param("name_of_service"))
    else {
        return result.fail("missing required parameter: service");
    };
    result.detail("service", service.clone());

    let windows_local = runner.is_local() && cfg!(windows);

    if let Some(expect_running) = spec.bool_param("running") {
        let running = if windows_local {
            windows_service_running(runner, &service)
        } else {
            unix_service_running(runner, &service)
        };
        result.detail("running", running);
        if running != expect_running {
            return result.fail(format!(
                "service {service} is {}, expected {}",
                state(running),
                state(expect_running)
            ));
        }
    }

    if let Some(expect_enabled) = spec.bool_param("enabled") {
        if windows_local {
            return result.fail("enabled checks are not supported for local Windows services");
        }
        let enabled = runner
            .run(&format!(
                "systemctl is-enabled {} 2>/dev/null",
                shell_quote(&service)
            ))
            .map(|o| o.stdout.trim() == "enabled")
            .unwrap_or(false);
        result.detail("enabled", enabled);
        if enabled != expect_enabled {
            return result.fail(format!(
                "service {service} is {}, expected {}",
                if enabled { "enabled" } else { "disabled" },
                if expect_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ));
        }
    }

    result.pass(format!("service {service} passed all checks"))
}

fn state(running: bool) -> &'static str {
    if running {
        "running"
    } else {
        "not running"
    }
}

fn unix_service_running(runner: &dyn Runner, service: &str) -> bool {
    // systemctl first, then a SysV/OpenRC-style fallback.
    let quoted = shell_quote(service);
    if let Ok(o) = runner.run(&format!("systemctl is-active {quoted} 2>/dev/null")) {
        if o.stdout.trim() == "active" {
            return true;
        }
        // systemctl present but service inactive
        if o.exit_code != 127 && !o.stdout.trim().is_empty() {
            return false;
        }
    }
    runner
        .run(&format!("service {quoted} status >/dev/null 2>&1"))
        .map(|o| o.exit_code == 0)
        .unwrap_or(false)
}

fn windows_service_running(runner: &dyn Runner, service: &str) -> bool {
    runner
        .run(&format!("sc query \"{service}\""))
        .map(|o| o.stdout.contains("RUNNING"))
        .unwrap_or(false)
}
