//! Run reports: aggregate check results and render terminal, JSON, or JUnit.

use crate::checks::CheckResult;
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct Summary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub title: String,
    pub target: String,
    pub timestamp: DateTime<Utc>,
    pub duration_seconds: f64,
    pub summary: Summary,
    pub checks: Vec<CheckResult>,
}

impl Report {
    pub fn new(title: &str, target: &str, checks: Vec<CheckResult>, duration: Duration) -> Self {
        let passed = checks.iter().filter(|c| c.passed).count();
        let failed = checks.len() - passed;
        Self {
            title: title.to_string(),
            target: target.to_string(),
            timestamp: Utc::now(),
            duration_seconds: duration.as_secs_f64(),
            summary: Summary {
                total: checks.len(),
                passed,
                failed,
            },
            checks,
        }
    }

    pub fn all_passed(&self) -> bool {
        self.summary.failed == 0
    }

    pub fn render(&self, format: &str, verbose: bool) -> anyhow::Result<String> {
        match format {
            "terminal" => Ok(self.render_terminal(verbose)),
            "json" => Ok(serde_json::to_string_pretty(self)?),
            "junit" => Ok(self.render_junit()),
            other => anyhow::bail!("unknown output format: {other} (terminal, json, junit)"),
        }
    }

    fn render_terminal(&self, verbose: bool) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "\n{} {}  (target: {})\n\n",
            "driftguard".bold(),
            self.title,
            self.target
        ));

        for check in &self.checks {
            let marker = if check.passed {
                "PASS".green().bold()
            } else {
                "FAIL".red().bold()
            };
            out.push_str(&format!("  {marker}  {}\n", check.name));
            if !check.passed || verbose {
                if !check.message.is_empty() {
                    out.push_str(&format!("        {}\n", check.message.dimmed()));
                }
                if verbose {
                    for (k, v) in &check.details {
                        out.push_str(&format!("        {k}: {v}\n"));
                    }
                }
            }
        }

        let summary = format!(
            "{} checks: {} passed, {} failed ({:.2}s)",
            self.summary.total, self.summary.passed, self.summary.failed, self.duration_seconds
        );
        out.push_str(&format!(
            "\n  {}\n",
            if self.all_passed() {
                summary.green().to_string()
            } else {
                summary.red().to_string()
            }
        ));
        out
    }

    /// JUnit XML so CI systems (GitHub Actions, GitLab, Jenkins) can render
    /// post-deployment checks as a test report.
    fn render_junit(&self) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str(&format!(
            "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"0\" time=\"{:.3}\" timestamp=\"{}\">\n",
            xml_escape(&self.title),
            self.summary.total,
            self.summary.failed,
            self.duration_seconds,
            self.timestamp.format("%Y-%m-%dT%H:%M:%S"),
        ));
        for check in &self.checks {
            out.push_str(&format!(
                "  <testcase name=\"{}\" classname=\"driftguard.{}\"",
                xml_escape(&check.name),
                xml_escape(&check.check_type),
            ));
            if check.passed {
                out.push_str("/>\n");
            } else {
                out.push_str(&format!(
                    ">\n    <failure message=\"{}\"/>\n  </testcase>\n",
                    xml_escape(&check.message),
                ));
            }
        }
        out.push_str("</testsuite>\n");
        out
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckResult;
    use crate::config::CheckSpec;
    use std::collections::BTreeMap;

    fn sample_checks() -> Vec<CheckResult> {
        let pass_spec = CheckSpec {
            name: "passing check".to_string(),
            check_type: "command".to_string(),
            params: BTreeMap::new(),
        };
        let fail_spec = CheckSpec {
            name: "failing <check>".to_string(),
            check_type: "file".to_string(),
            params: BTreeMap::new(),
        };
        vec![
            CheckResult::new(&pass_spec).pass("ok"),
            CheckResult::new(&fail_spec).fail("missing & broken"),
        ]
    }

    #[test]
    fn summary_counts() {
        let report = Report::new("t", "local", sample_checks(), Duration::from_secs(1));
        assert_eq!(report.summary.total, 2);
        assert_eq!(report.summary.passed, 1);
        assert_eq!(report.summary.failed, 1);
        assert!(!report.all_passed());
    }

    #[test]
    fn junit_escapes_and_reports_failures() {
        let report = Report::new("suite", "local", sample_checks(), Duration::from_secs(1));
        let xml = report.render_junit();
        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("failing &lt;check&gt;"));
        assert!(xml.contains("missing &amp; broken"));
        assert!(!xml.contains("<check>"));
    }

    #[test]
    fn json_round_trips() {
        let report = Report::new("suite", "local", sample_checks(), Duration::from_secs(1));
        let json = report.render("json", false).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["summary"]["total"], 2);
        assert_eq!(parsed["checks"][0]["passed"], true);
    }
}
