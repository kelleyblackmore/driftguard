//! Process checks: is a process running, and how many instances.

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::{shell_quote, Runner};

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(process) = spec
        .str_param("process")
        .or_else(|| spec.str_param("process_name"))
    else {
        return result.fail("missing required parameter: process");
    };
    result.detail("process", process.clone());

    let count = if runner.is_local() && cfg!(windows) {
        windows_process_count(runner, &process)
    } else {
        unix_process_count(runner, &process)
    };
    result.detail("count", count);

    let expect_running = spec.bool_param("running").unwrap_or(true);
    let running = count > 0;
    if running != expect_running {
        return result.fail(format!(
            "process {process} has {count} instance(s), expected it to be {}",
            if expect_running { "running" } else { "absent" }
        ));
    }

    if let Some(expected) = spec.int_param("count") {
        if count != expected {
            return result.fail(format!(
                "process {process} has {count} instance(s), expected exactly {expected}"
            ));
        }
    }
    if let Some(min) = spec.int_param("min_count") {
        if count < min {
            return result.fail(format!(
                "process {process} has {count} instance(s), expected at least {min}"
            ));
        }
    }
    if let Some(max) = spec.int_param("max_count") {
        if count > max {
            return result.fail(format!(
                "process {process} has {count} instance(s), expected at most {max}"
            ));
        }
    }

    result.pass(format!("process {process} passed all checks"))
}

fn unix_process_count(runner: &dyn Runner, process: &str) -> i64 {
    let quoted = shell_quote(process);
    if let Ok(o) = runner.run(&format!("pgrep -c -x {quoted} 2>/dev/null")) {
        if o.exit_code == 0 || o.exit_code == 1 {
            if let Ok(n) = o.stdout.trim().parse::<i64>() {
                return n;
            }
            if o.exit_code == 1 {
                return 0;
            }
        }
    }
    // Fallback: parse ps output for exact command-name matches.
    runner
        .run("ps -e -o comm= 2>/dev/null")
        .map(|o| {
            o.stdout
                .lines()
                .filter(|line| {
                    let name = line.trim().rsplit('/').next().unwrap_or(line.trim());
                    name == process
                })
                .count() as i64
        })
        .unwrap_or(0)
}

fn windows_process_count(runner: &dyn Runner, process: &str) -> i64 {
    let image = if process.to_lowercase().ends_with(".exe") {
        process.to_string()
    } else {
        format!("{process}.exe")
    };
    runner
        .run(&format!(
            "tasklist /FI \"IMAGENAME eq {image}\" /NH /FO CSV"
        ))
        .map(|o| {
            o.stdout
                .lines()
                .filter(|l| l.to_lowercase().contains(&image.to_lowercase()))
                .count() as i64
        })
        .unwrap_or(0)
}
