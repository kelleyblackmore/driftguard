//! driftguard — post-IaC server verification.
//!
//! After Terraform, Ansible, or any other provisioning tool runs, driftguard
//! executes a YAML-defined suite of checks against the deployed host (locally
//! or over SSH) and reports whether reality matches intent.

mod checks;
mod config;
mod report;
mod runner;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use runner::{LocalRunner, Runner, SshRunner};

#[derive(Parser)]
#[command(name = "driftguard", version, about, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run checks from a YAML configuration against a target
    Run {
        /// Path to the checks YAML file
        config: PathBuf,
        /// Remote target as [user@]host; omit to check the local machine
        #[arg(long, short = 'H')]
        host: Option<String>,
        /// SSH port
        #[arg(long, short)]
        port: Option<u16>,
        /// SSH identity file
        #[arg(long, short)]
        key_file: Option<PathBuf>,
        /// Accept previously-unseen SSH host keys, for freshly provisioned
        /// hosts in CI (StrictHostKeyChecking=accept-new)
        #[arg(long)]
        accept_new_host_key: bool,
        /// Output format
        #[arg(long, short, default_value = "terminal", value_parser = ["terminal", "json", "junit"])]
        output_format: String,
        /// Write output to a file instead of stdout
        #[arg(long, short = 'f')]
        output_file: Option<PathBuf>,
        /// Override or define a variable (repeatable): --var KEY=VALUE
        #[arg(long = "var", value_name = "KEY=VALUE")]
        vars: Vec<String>,
        /// Show details for passing checks too
        #[arg(long, short)]
        verbose: bool,
    },
    /// Parse and validate a configuration without running it
    Validate {
        /// Path to the checks YAML file
        config: PathBuf,
    },
    /// Write an example checks file to get started
    Init {
        /// Where to write the example (default: driftguard.yaml)
        #[arg(default_value = "driftguard.yaml")]
        path: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            2
        }
    };
    std::process::exit(exit_code);
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Commands::Run {
            config,
            host,
            port,
            key_file,
            accept_new_host_key,
            output_format,
            output_file,
            vars,
            verbose,
        } => {
            let overrides = parse_vars(&vars)?;
            let cfg = config::load(&config, &overrides)?;

            let runner: Box<dyn Runner> = match &host {
                Some(target) => {
                    Box::new(SshRunner::new(target, port, key_file, accept_new_host_key))
                }
                None => Box::new(LocalRunner),
            };

            let started = Instant::now();
            let results: Vec<_> = cfg
                .checks
                .iter()
                .map(|spec| checks::run_check(runner.as_ref(), spec))
                .collect();

            let title = cfg.title.as_deref().unwrap_or("driftguard checks");
            let report = report::Report::new(title, &runner.target(), results, started.elapsed());

            let rendered = report.render(&output_format, verbose)?;
            match output_file {
                Some(path) => {
                    std::fs::write(&path, &rendered)
                        .with_context(|| format!("failed to write {}", path.display()))?;
                    eprintln!("results written to {}", path.display());
                }
                None => print!("{rendered}"),
            }

            Ok(if report.all_passed() { 0 } else { 1 })
        }

        Commands::Validate { config } => {
            let cfg = config::load(&config, &BTreeMap::new())?;
            println!("{}: OK ({} checks)", config.display(), cfg.checks.len());
            if let Some(description) = &cfg.description {
                println!("  {description}");
            }
            for check in &cfg.checks {
                println!("  [{}] {}", check.check_type, check.name);
            }
            Ok(0)
        }

        Commands::Init { path } => {
            if path.exists() {
                anyhow::bail!("{} already exists, not overwriting", path.display());
            }
            std::fs::write(&path, EXAMPLE_CONFIG)
                .with_context(|| format!("failed to write {}", path.display()))?;
            println!("wrote example checks to {}", path.display());
            Ok(0)
        }
    }
}

fn parse_vars(vars: &[String]) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for pair in vars {
        let (key, value) = pair
            .split_once('=')
            .with_context(|| format!("invalid --var '{pair}', expected KEY=VALUE"))?;
        map.insert(key.trim().to_string(), value.to_string());
    }
    Ok(map)
}

const EXAMPLE_CONFIG: &str = r#"---
# driftguard checks — run after your IaC applies to verify the deployment.
# Example: driftguard run driftguard.yaml --host admin@$(terraform output -raw ip)
title: Web server deployment checks

variables:
  WEB_ROOT: /var/www/html

checks:
  - name: nginx package installed
    type: package
    package: nginx
    installed: true

  - name: nginx service running and enabled
    type: service
    service: nginx
    running: true
    enabled: true

  - name: web root exists
    type: directory
    path: "{{ WEB_ROOT }}"
    exists: true

  - name: nginx listening on 80
    type: port
    port: 80
    protocol: tcp
    listening: true

  - name: nginx config is valid
    type: command
    command: nginx -t
    exit_status: 0

  - name: app config has expected values
    type: config
    path: /etc/myapp/config.json
    format: json
    has_key: service.enabled
    has_value:
      service.port: 8080
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vars_accepts_key_value_pairs() {
        let vars = parse_vars(&["A=1".to_string(), "B=x=y".to_string()]).unwrap();
        assert_eq!(vars["A"], "1");
        assert_eq!(vars["B"], "x=y");
    }

    #[test]
    fn parse_vars_rejects_missing_equals() {
        assert!(parse_vars(&["NOPE".to_string()]).is_err());
    }

    #[test]
    fn example_config_parses() {
        let doc: serde_yaml::Value = serde_yaml::from_str(EXAMPLE_CONFIG).unwrap();
        assert!(doc.get("checks").unwrap().as_sequence().unwrap().len() >= 5);
    }
}
