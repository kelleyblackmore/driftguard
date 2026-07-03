//! Command execution against the target under test: the local machine or a
//! remote host over SSH.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

/// Output of a command executed on the target.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait Runner {
    /// Human-readable target name for reports ("local" or "user@host").
    fn target(&self) -> String;

    fn is_local(&self) -> bool;

    /// Run a command through the target's shell. Commands come from the
    /// operator's own check configuration and are executed verbatim; shell
    /// features (pipes, redirects) are supported intentionally.
    fn run(&self, command: &str) -> Result<CommandOutput>;

    /// Whether a path exists on the target.
    fn file_exists(&self, path: &str) -> bool;

    /// Read a file's contents from the target.
    fn read_file(&self, path: &str) -> Result<String>;
}

/// Runs checks on the local machine.
pub struct LocalRunner;

impl Runner for LocalRunner {
    fn target(&self) -> String {
        "local".to_string()
    }

    fn is_local(&self) -> bool {
        true
    }

    fn run(&self, command: &str) -> Result<CommandOutput> {
        #[cfg(windows)]
        let output = {
            use std::os::windows::process::CommandExt;
            // raw_arg passes the command line to cmd verbatim; the default
            // argument quoting would corrupt commands containing quotes.
            Command::new("cmd")
                .arg("/C")
                .raw_arg(command)
                .output()
                .context("failed to spawn cmd")?
        };

        #[cfg(not(windows))]
        let output = Command::new("sh")
            .args(["-c", command])
            .output()
            .context("failed to spawn sh")?;

        Ok(CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn file_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn read_file(&self, path: &str) -> Result<String> {
        std::fs::read_to_string(path).with_context(|| format!("failed to read {path}"))
    }
}

/// Runs checks on a remote host by shelling out to the system OpenSSH client.
///
/// Using the system `ssh` keeps driftguard dependency-free (no native TLS or
/// libssh2 build requirements), honors the user's ~/.ssh/config, and works
/// identically on Linux, macOS, and Windows 10+.
pub struct SshRunner {
    pub target: String,
    pub port: Option<u16>,
    pub key_file: Option<PathBuf>,
    pub connect_timeout_secs: u32,
}

impl SshRunner {
    pub fn new(target: &str, port: Option<u16>, key_file: Option<PathBuf>) -> Self {
        Self {
            target: target.to_string(),
            port,
            key_file,
            connect_timeout_secs: 10,
        }
    }

    fn ssh_command(&self, remote_command: &str) -> Command {
        let mut cmd = Command::new("ssh");
        // BatchMode fails fast instead of hanging on a password prompt;
        // driftguard is built to run unattended in CI pipelines.
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!("ConnectTimeout={}", self.connect_timeout_secs));
        if let Some(port) = self.port {
            cmd.arg("-p").arg(port.to_string());
        }
        if let Some(key) = &self.key_file {
            cmd.arg("-i").arg(key);
        }
        cmd.arg(&self.target).arg(remote_command);
        cmd
    }
}

impl Runner for SshRunner {
    fn target(&self) -> String {
        self.target.clone()
    }

    fn is_local(&self) -> bool {
        false
    }

    fn run(&self, command: &str) -> Result<CommandOutput> {
        let output = self
            .ssh_command(command)
            .output()
            .context("failed to spawn ssh (is an OpenSSH client installed?)")?;

        Ok(CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn file_exists(&self, path: &str) -> bool {
        self.run(&format!("test -e {}", shell_quote(path)))
            .map(|o| o.exit_code == 0)
            .unwrap_or(false)
    }

    fn read_file(&self, path: &str) -> Result<String> {
        let out = self.run(&format!("cat {}", shell_quote(path)))?;
        if out.exit_code != 0 {
            anyhow::bail!("failed to read {path}: {}", out.stderr.trim());
        }
        Ok(out.stdout)
    }
}

/// POSIX single-quote escaping for values interpolated into remote commands.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_runner_executes_and_captures_output() {
        let out = LocalRunner.run("echo driftguard").unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("driftguard"));
    }

    #[test]
    fn local_runner_reports_nonzero_exit() {
        let out = LocalRunner.run("exit 3").unwrap();
        assert_eq!(out.exit_code, 3);
    }

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn ssh_command_includes_options() {
        let runner = SshRunner::new("admin@example.com", Some(2222), None);
        let cmd = runner.ssh_command("uptime");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
        assert!(args.contains(&"admin@example.com".to_string()));
        assert!(args.contains(&"uptime".to_string()));
    }
}
