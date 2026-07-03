//! Port checks: is something listening on a TCP/UDP port.

use super::CheckResult;
use crate::config::CheckSpec;
use crate::runner::Runner;
use std::net::TcpStream;
use std::time::Duration;

pub fn run(runner: &dyn Runner, spec: &CheckSpec) -> CheckResult {
    let mut result = CheckResult::new(spec);

    let Some(port) = spec.int_param("port").filter(|p| (1..=65535).contains(p)) else {
        return result.fail("missing or invalid required parameter: port (1-65535)");
    };
    result.detail("port", port);

    let protocol = spec
        .str_param("protocol")
        .unwrap_or_else(|| "tcp".to_string())
        .to_lowercase();
    result.detail("protocol", protocol.clone());
    if protocol != "tcp" && protocol != "udp" {
        return result.fail(format!("unsupported protocol: {protocol}"));
    }

    let expect_listening = spec.bool_param("listening").unwrap_or(true);

    let listening = if runner.is_local() && protocol == "tcp" {
        // Fast path: a TCP connect to loopback proves a listener locally.
        local_tcp_listening(port as u16)
            || listening_in_socket_table(runner, port, &protocol).unwrap_or(false)
    } else {
        match listening_in_socket_table(runner, port, &protocol) {
            Some(v) => v,
            None => {
                return result
                    .fail("could not inspect listening sockets (ss/netstat unavailable on target)")
            }
        }
    };
    result.detail("listening", listening);

    if listening != expect_listening {
        return result.fail(format!(
            "{protocol} port {port} is {}, expected {}",
            if listening {
                "listening"
            } else {
                "not listening"
            },
            if expect_listening {
                "listening"
            } else {
                "not listening"
            }
        ));
    }

    result.pass(format!("{protocol} port {port} passed all checks"))
}

fn local_tcp_listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(2),
    )
    .is_ok()
}

/// Look for the port in the target's socket table via ss, netstat, or (on
/// Windows) netstat -an. Returns None when no tool produced usable output.
fn listening_in_socket_table(runner: &dyn Runner, port: i64, protocol: &str) -> Option<bool> {
    let flag = if protocol == "tcp" { "t" } else { "u" };
    let commands = if runner.is_local() && cfg!(windows) {
        vec![format!("netstat -an -p {protocol}")]
    } else {
        vec![
            format!("ss -{flag}ln 2>/dev/null"),
            format!("netstat -{flag}ln 2>/dev/null"),
        ]
    };

    for cmd in commands {
        if let Ok(out) = runner.run(&cmd) {
            if out.exit_code == 0 && !out.stdout.trim().is_empty() {
                return Some(table_has_listener(&out.stdout, port));
            }
        }
    }
    None
}

fn table_has_listener(table: &str, port: i64) -> bool {
    let suffix = format!(":{port}");
    table.lines().any(|line| {
        let l = line.to_uppercase();
        let looks_listening = l.contains("LISTEN") || l.contains("UNCONN") || l.starts_with("UDP");
        if !looks_listening {
            return false;
        }
        // Local-address column ends with :PORT
        line.split_whitespace()
            .any(|col| col.ends_with(&suffix) && (col.contains(':') || col.contains('.')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ss_output() {
        let table = "LISTEN 0 128 0.0.0.0:22 0.0.0.0:*\nLISTEN 0 511 *:80 *:*";
        assert!(table_has_listener(table, 22));
        assert!(table_has_listener(table, 80));
        assert!(!table_has_listener(table, 443));
    }

    #[test]
    fn parses_windows_netstat_output() {
        let table = "  TCP    0.0.0.0:135    0.0.0.0:0    LISTENING\n  TCP    [::]:445    [::]:0    LISTENING";
        assert!(table_has_listener(table, 135));
        assert!(table_has_listener(table, 445));
        assert!(!table_has_listener(table, 8080));
    }

    #[test]
    fn port_number_must_match_exactly() {
        let table = "LISTEN 0 128 0.0.0.0:2222 0.0.0.0:*";
        assert!(!table_has_listener(table, 22));
        assert!(table_has_listener(table, 2222));
    }
}
