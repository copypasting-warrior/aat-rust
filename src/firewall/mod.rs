// Lightweight firewall scanner that prefers UFW, then firewalld, then nftables, then iptables.

use std::process::Command;

#[derive(Clone, Debug)]
pub struct FirewallSnapshot {
    pub backend: String,
    pub enabled: bool,
    pub default_input_policy: String,
    pub default_output_policy: String,
    pub rules_count: usize,
    pub open_ports: Vec<String>,
    pub status_line: String,
    pub note: Option<String>,
}

impl FirewallSnapshot {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            backend: "none".to_string(),
            enabled: false,
            default_input_policy: "--".to_string(),
            default_output_policy: "--".to_string(),
            rules_count: 0,
            open_ports: Vec::new(),
            status_line: "Firewall not detected".to_string(),
            note: Some(reason.into()),
        }
    }
}

pub fn scan_firewall() -> FirewallSnapshot {
    if let Some(ufw) = scan_ufw() {
        return ufw;
    }

    if let Some(fw) = scan_firewalld() {
        return fw;
    }

    if let Some(nft) = scan_nftables() {
        return nft;
    }

    if let Some(ipt) = scan_iptables() {
        return ipt;
    }

    if let Some(service_hint) = detect_active_firewall_service() {
        return service_hint;
    }

    FirewallSnapshot::unavailable("Could not read ufw, nft, or iptables state")
}

fn scan_ufw() -> Option<FirewallSnapshot> {
    let result = run_command("ufw", &["status", "numbered"])?;
    let text = result.primary_text();
    if !text.to_ascii_lowercase().contains("status:") {
        return None;
    }

    let mut enabled = false;
    let mut status_line = String::from("Status: unknown");
    let mut default_policy = String::from("--");
    let mut rules_count = 0usize;
    let mut open_ports: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();

        if lower.starts_with("status:") {
            enabled = lower.contains("active");
            status_line = trimmed.to_string();
            continue;
        }

        if lower.starts_with("default:") {
            default_policy = trimmed
                .trim_start_matches("Default:")
                .trim()
                .to_string();
            continue;
        }

        if !looks_like_ufw_rule_line(trimmed) {
            continue;
        }

        rules_count += 1;
        if line_has_allow(trimmed) {
            if let Some(port) = extract_ufw_port(trimmed) {
                if !open_ports.iter().any(|p| p == &port) {
                    open_ports.push(port);
                }
            }
        }
    }

    Some(FirewallSnapshot {
        backend: "ufw".to_string(),
        enabled,
        default_input_policy: default_policy.clone(),
        default_output_policy: default_policy,
        rules_count,
        open_ports,
        status_line,
        note: result.permission_note(),
    })
}

fn scan_firewalld() -> Option<FirewallSnapshot> {
    let state_result = run_command("firewall-cmd", &["--state"])?;
    let state_text = state_result.primary_text().to_ascii_lowercase();
    if !state_text.contains("running") && !state_text.contains("not running") {
        return None;
    }

    let enabled = state_text.contains("running") && !state_text.contains("not running");

    let default_zone = run_command("firewall-cmd", &["--get-default-zone"])
        .map(|r| r.primary_text().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "--".to_string());

    let open_ports = run_command("firewall-cmd", &["--list-ports"])
        .map(|r| {
            r.primary_text()
                .split_whitespace()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    Some(FirewallSnapshot {
        backend: "firewalld".to_string(),
        enabled,
        default_input_policy: default_zone.clone(),
        default_output_policy: default_zone,
        rules_count: open_ports.len(),
        open_ports,
        status_line: if enabled {
            "firewalld active".to_string()
        } else {
            "firewalld inactive".to_string()
        },
        note: state_result.permission_note(),
    })
}

fn scan_nftables() -> Option<FirewallSnapshot> {
    let result = run_command("nft", &["list", "ruleset"])?;
    let text = result.primary_text();

    if !result.success && text.trim().is_empty() {
        return None;
    }

    if text.trim().is_empty() {
        return Some(FirewallSnapshot {
            backend: "nftables".to_string(),
            enabled: true,
            default_input_policy: "--".to_string(),
            default_output_policy: "--".to_string(),
            rules_count: 0,
            open_ports: Vec::new(),
            status_line: "nftables detected".to_string(),
            note: result.permission_note().or_else(|| Some("Could not read nft ruleset".to_string())),
        });
    }

    let input_policy = extract_nft_chain_policy(&text, "input").unwrap_or_else(|| "--".to_string());
    let output_policy = extract_nft_chain_policy(&text, "output").unwrap_or_else(|| "--".to_string());

    let mut rules_count = 0usize;
    let mut open_ports: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("chain ")
            || trimmed.starts_with("table ")
            || trimmed.starts_with('}')
            || trimmed.is_empty()
        {
            continue;
        }

        // Count non-structural lines as rules to avoid showing zero on valid rulesets
        // that do not explicitly contain accept/drop/reject tokens.
        if !trimmed.starts_with("type ") {
            rules_count += 1;
        }

        if line_has_accept(trimmed) {
            for p in extract_dports(trimmed) {
                if !open_ports.iter().any(|v| v == &p) {
                    open_ports.push(p);
                }
            }
        }
    }

    Some(FirewallSnapshot {
        backend: "nftables".to_string(),
        enabled: true,
        default_input_policy: input_policy.clone(),
        default_output_policy: output_policy,
        rules_count,
        open_ports,
        status_line: format!("nftables active (input: {})", input_policy),
        note: result.permission_note(),
    })
}

fn scan_iptables() -> Option<FirewallSnapshot> {
    let result = run_command("iptables", &["-S"])?;
    let text = result.primary_text();

    if !result.success && text.trim().is_empty() {
        return None;
    }

    if text.trim().is_empty() {
        return Some(FirewallSnapshot {
            backend: "iptables".to_string(),
            enabled: true,
            default_input_policy: "--".to_string(),
            default_output_policy: "--".to_string(),
            rules_count: 0,
            open_ports: Vec::new(),
            status_line: "iptables detected".to_string(),
            note: result.permission_note().or_else(|| Some("Could not read iptables rules".to_string())),
        });
    }

    let mut input_policy = String::from("--");
    let mut output_policy = String::from("--");
    let mut rules_count = 0usize;
    let mut open_ports: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(policy) = trimmed.strip_prefix("-P INPUT ") {
            input_policy = policy.trim().to_string();
        }
        if let Some(policy) = trimmed.strip_prefix("-P OUTPUT ") {
            output_policy = policy.trim().to_string();
        }

        if trimmed.starts_with("-A ") {
            rules_count += 1;
            if line_has_accept(trimmed) {
                if let Some(port) = extract_port_after_keyword(trimmed, "--dport") {
                    if !open_ports.iter().any(|p| p == &port) {
                        open_ports.push(port);
                    }
                }
            }
        }
    }

    Some(FirewallSnapshot {
        backend: "iptables".to_string(),
        enabled: true,
        default_input_policy: input_policy.clone(),
        default_output_policy: output_policy,
        rules_count,
        open_ports,
        status_line: format!("iptables active (input: {})", input_policy),
        note: result.permission_note(),
    })
}

fn detect_active_firewall_service() -> Option<FirewallSnapshot> {
    let candidates = ["ufw", "nftables", "firewalld"];
    for service in candidates {
        let out = run_command("systemctl", &["is-active", service])?;
        if out.primary_text().trim() == "active" {
            return Some(FirewallSnapshot {
                backend: service.to_string(),
                enabled: true,
                default_input_policy: "--".to_string(),
                default_output_policy: "--".to_string(),
                rules_count: 0,
                open_ports: Vec::new(),
                status_line: format!("{} service is active", service),
                note: Some("Rules could not be read from user context".to_string()),
            });
        }
    }

    None
}

#[derive(Clone, Debug)]
struct CommandResult {
    success: bool,
    stdout: String,
    stderr: String,
}

impl CommandResult {
    fn primary_text(&self) -> String {
        if !self.stdout.trim().is_empty() {
            return self.stdout.clone();
        }
        self.stderr.clone()
    }

    fn permission_note(&self) -> Option<String> {
        let low = self.stderr.to_ascii_lowercase();
        if low.contains("permission denied") || low.contains("operation not permitted") {
            return Some("Permission denied while reading full firewall rules".to_string());
        }
        None
    }
}

fn run_command(cmd: &str, args: &[&str]) -> Option<CommandResult> {
    let output = Command::new(cmd).args(args).output().ok()?;
    Some(CommandResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn looks_like_ufw_rule_line(line: &str) -> bool {
    line.contains("ALLOW") || line.contains("DENY") || line.contains("REJECT")
}

fn line_has_allow(line: &str) -> bool {
    line.split_whitespace().any(|w| w == "ALLOW")
}

fn line_has_accept(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains(" accept") || lower.ends_with("accept")
}

fn extract_ufw_port(line: &str) -> Option<String> {
    let mut tokens = line.split_whitespace();
    let first = tokens.next()?;

    if first.starts_with('[') {
        return tokens
            .next()
            .map(|v| v.trim_matches(|c| c == '[' || c == ']').to_string());
    }

    Some(first.to_string())
}

fn extract_nft_chain_policy(text: &str, chain_name: &str) -> Option<String> {
    let target = format!("chain {}", chain_name);
    let mut in_chain = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("chain ") {
            in_chain = trimmed.starts_with(&target);
            if in_chain {
                if let Some(policy) = extract_port_after_keyword(trimmed, "policy") {
                    return Some(policy.trim_end_matches(';').to_string());
                }
            }
            continue;
        }

        if in_chain {
            if trimmed.starts_with('}') {
                break;
            }
            if let Some(policy) = extract_port_after_keyword(trimmed, "policy") {
                return Some(policy.trim_end_matches(';').to_string());
            }
        }
    }

    None
}

fn extract_dports(line: &str) -> Vec<String> {
    if !line.contains("dport") {
        return Vec::new();
    }

    if line.contains("dport {") {
        if let Some(start) = line.find('{') {
            if let Some(end) = line[start + 1..].find('}') {
                return line[start + 1..start + 1 + end]
                    .split(',')
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect();
            }
        }
    }

    extract_port_after_keyword(line, "dport")
        .map(|v| vec![v])
        .unwrap_or_default()
}

fn extract_port_after_keyword(line: &str, keyword: &str) -> Option<String> {
    let mut iter = line.split_whitespace();
    while let Some(tok) = iter.next() {
        if tok == keyword {
            return iter.next().map(|v| v.to_string());
        }
    }
    None
}
