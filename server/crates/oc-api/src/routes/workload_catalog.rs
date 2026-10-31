//! Workload Protection's catalog: the default agent policies, the agent rules in them (SECL
//! expressions the Datadog Agent's runtime security module evaluates), the content packs that group
//! them, the `workload_security` detection rules on the events they capture and the
//! `workload_activity` finding rules. All of it is ObserveCraft's own; the tactic and technique ids
//! are MITRE ATT&CK's.

/// The default agent policies: (key, name, description).
pub const POLICIES: [(&str, &str, &str); 3] = [
    ("threat-detection", "Threat detection", "Detects intrusions, malware and attacker tooling running on hosts and in containers."),
    ("compliance-fim", "Compliance & file integrity monitoring", "Tracks changes to the system files compliance frameworks such as PCI DSS and HIPAA ask to watch."),
    ("best-practices", "Best practices", "Collects runtime activity that weakens a workload's security posture."),
];

/// MITRE ATT&CK enterprise tactics, in kill-chain order: (id, name).
pub const TACTICS: [(&str, &str); 14] = [
    ("TA0043", "Reconnaissance"),
    ("TA0042", "Resource Development"),
    ("TA0001", "Initial Access"),
    ("TA0002", "Execution"),
    ("TA0003", "Persistence"),
    ("TA0004", "Privilege Escalation"),
    ("TA0005", "Defense Evasion"),
    ("TA0006", "Credential Access"),
    ("TA0007", "Discovery"),
    ("TA0008", "Lateral Movement"),
    ("TA0009", "Collection"),
    ("TA0011", "Command and Control"),
    ("TA0010", "Exfiltration"),
    ("TA0040", "Impact"),
];

/// A default agent rule.
pub struct AgentRule {
    /// The rule id the Agent reports (`@agent.rule_id`).
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub expression: &'static str,
    /// The policy key ([`POLICIES`]).
    pub policy: &'static str,
    /// Tactic id ([`TACTICS`]) and technique, or "" for posture rules.
    pub tactic: &'static str,
    pub technique: &'static str,
    /// The content pack ([`PACKS`]), or "".
    pub pack: &'static str,
    /// The severity of the detection rule raised on it.
    pub severity: &'static str,
}

// A table row: the fields in order read better than a struct literal per rule.
#[allow(clippy::too_many_arguments)]
const fn rule(name: &'static str, title: &'static str, description: &'static str, expression: &'static str, policy: &'static str, tactic: &'static str, technique: &'static str, pack: &'static str, severity: &'static str) -> AgentRule {
    AgentRule { name, title, description, expression, policy, tactic, technique, pack, severity }
}

const T: &str = "threat-detection";
const C: &str = "compliance-fim";
const B: &str = "best-practices";

pub const AGENT_RULES: &[AgentRule] = &[
    // ---------------------------------------------------------------- threat detection
    rule("web_server_spawned_shell", "Web server process spawned a shell", "A web server or application runtime started a shell, a common sign of a web shell or command injection.",
        r#"exec.file.name in ["sh", "bash", "dash", "zsh"] && process.parent.file.name in ["nginx", "httpd", "php-fpm", "java", "node", "python3"]"#, T, "TA0001", "T1190", "container-drift", "high"),
    rule("shell_in_container", "Interactive shell started in a container", "A shell started in a running container by something other than a shell or the container runtime.",
        r#"exec.file.name in ["sh", "bash", "dash", "zsh", "ash"] && container.id != "" && process.parent.file.name not in ["sh", "bash", "dash", "zsh", "ash", "runc", "containerd-shim"]"#, T, "TA0002", "T1059.004", "container-drift", "medium"),
    rule("package_manager_in_container", "Package manager run in a container", "Software was installed in a running container instead of its image.",
        r#"exec.file.name in ["apt", "apt-get", "yum", "dnf", "apk", "pip", "pip3", "npm"] && container.id != """#, T, "TA0002", "T1072", "container-drift", "low"),
    rule("downloaded_binary_executed", "Binary executed from a temporary directory", "A program ran from /tmp or /dev/shm, where droppers usually write their payloads.",
        r#"exec.file.path =~ "/tmp/**" || exec.file.path =~ "/dev/shm/**""#, T, "TA0002", "T1204.002", "attacker-tools", "medium"),
    rule("cron_job_modified", "Cron job created or modified", "A file under the cron directories was written.",
        r#"open.file.path =~ "/etc/cron*/**" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, T, "TA0003", "T1053.003", "host-persistence", "medium"),
    rule("ssh_authorized_keys_modified", "SSH authorized keys modified", "An authorized_keys file was written, which grants SSH access to whoever holds the key.",
        r#"open.file.name == "authorized_keys" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, T, "TA0003", "T1098.004", "host-persistence", "high"),
    rule("user_account_created", "User account created", "A user account was added on the host.",
        r#"exec.file.name in ["useradd", "adduser"]"#, T, "TA0003", "T1136.001", "host-persistence", "medium"),
    rule("kernel_module_loaded", "Kernel module loaded", "A kernel module was loaded at runtime, which rootkits use to hide.",
        r#"load_module.name != "" && process.file.name not in ["systemd-modules-load"]"#, T, "TA0003", "T1547.006", "host-persistence", "high"),
    rule("sudoers_modified", "Sudoers policy modified", "The sudoers file or a file in /etc/sudoers.d was written.",
        r#"(open.file.path == "/etc/sudoers" || open.file.path =~ "/etc/sudoers.d/*") && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, T, "TA0004", "T1548.003", "host-persistence", "medium"),
    rule("setuid_bit_set", "Setuid bit set on a file", "A file was made setuid, which lets any user run it with its owner's rights.",
        r#"chmod.file.destination.mode & S_ISUID > 0"#, T, "TA0004", "T1548.001", "container-escape", "high"),
    rule("nsenter_in_container", "Namespace entered from a container", "nsenter ran in a container, a way to reach the host's namespaces.",
        r#"exec.file.name == "nsenter" && container.id != """#, T, "TA0004", "T1611", "container-escape", "critical"),
    rule("process_memory_injection", "Process memory written with ptrace", "A process wrote into another process's memory.",
        r#"ptrace.request in [PTRACE_POKETEXT, PTRACE_POKEDATA, PTRACE_POKEUSR]"#, T, "TA0005", "T1055.008", "container-escape", "high"),
    rule("shell_history_cleared", "Shell history deleted", "A shell history file was deleted.",
        r#"unlink.file.name in [".bash_history", ".zsh_history", ".ash_history"]"#, T, "TA0005", "T1070.003", "anti-forensics", "medium"),
    rule("system_log_deleted", "System log file deleted", "A file under /var/log was deleted.",
        r#"unlink.file.path =~ "/var/log/**""#, T, "TA0005", "T1070.002", "anti-forensics", "medium"),
    rule("firewall_rules_flushed", "Firewall rules flushed", "The host firewall's rules were flushed.",
        r#"exec.file.name in ["iptables", "ip6tables", "nft"] && process.args_flags in ["F", "flush"]"#, T, "TA0005", "T1562.004", "anti-forensics", "high"),
    rule("compiler_in_container", "Compiler run in a container", "Code was compiled inside a running container, which attackers do to build tools on target.",
        r#"exec.file.name in ["gcc", "cc", "clang", "go", "rustc", "make"] && container.id != """#, T, "TA0005", "T1027.004", "container-drift", "low"),
    rule("shadow_file_read", "Password hashes read", "/etc/shadow was opened by a program that doesn't manage passwords.",
        r#"open.file.path == "/etc/shadow" && process.file.name not in ["passwd", "chpasswd", "useradd", "usermod", "sshd", "login", "su", "sudo"]"#, T, "TA0006", "T1003.008", "credential-theft", "high"),
    rule("service_account_token_read", "Kubernetes service account token read", "A shell tool read the pod's service account token.",
        r#"open.file.path =~ "/var/run/secrets/kubernetes.io/serviceaccount/*" && process.file.name in ["cat", "curl", "wget", "sh", "bash"]"#, T, "TA0006", "T1528", "credential-theft", "medium"),
    rule("imds_v1_request", "Cloud metadata reached without a session token", "A process queried the instance metadata service with IMDSv1, which SSRF attacks rely on to steal credentials.",
        r#"imds.cloud_provider == "aws" && imds.aws.is_imds_v2 == false && imds.type == "request""#, T, "TA0006", "T1552.005", "cloud-metadata", "medium"),
    rule("network_scanner_executed", "Network scanner executed", "A port or network scanner ran.",
        r#"exec.file.name in ["nmap", "masscan", "zmap"]"#, T, "TA0007", "T1046", "attacker-tools", "medium"),
    rule("ssh_client_in_container", "SSH client run in a container", "ssh or scp ran in a container, which rarely needs to reach other machines that way.",
        r#"exec.file.name in ["ssh", "scp"] && container.id != """#, T, "TA0008", "T1021.004", "attacker-tools", "low"),
    rule("sensitive_files_archived", "System configuration archived", "An archiver packed files from /etc.",
        r#"exec.file.name in ["tar", "zip", "7z"] && process.argv in [~"/etc/*"]"#, T, "TA0009", "T1560.001", "credential-theft", "low"),
    rule("reverse_shell", "Reverse shell", "A shell redirected its input and output to a network socket.",
        r#"exec.file.name in ["sh", "bash"] && process.args =~ "*/dev/tcp/*""#, T, "TA0011", "T1059.004", "attacker-tools", "critical"),
    rule("netcat_executed", "Netcat-style tool executed", "A raw socket relay ran.",
        r#"exec.file.name in ["nc", "ncat", "netcat", "socat"]"#, T, "TA0011", "T1095", "attacker-tools", "medium"),
    rule("file_uploaded_with_curl", "File uploaded with curl", "curl uploaded a file to a remote server.",
        r#"exec.file.name == "curl" && process.argv in ["-T", "--upload-file", "-F"]"#, T, "TA0010", "T1048", "attacker-tools", "medium"),
    rule("cryptominer_executed", "Cryptocurrency miner executed", "A known miner ran or a process connected to a mining pool protocol.",
        r#"exec.file.name in ["xmrig", "minerd", "cpuminer"] || process.args =~ "*stratum+tcp://*""#, T, "TA0040", "T1496", "cryptomining", "high"),
    rule("mining_pool_dns", "Mining pool domain resolved", "A process resolved a domain used by cryptocurrency mining pools.",
        r#"dns.question.name =~ "*pool*" && dns.question.name =~ "*xmr*""#, T, "TA0040", "T1496", "cryptomining", "medium"),
    // ---------------------------------------------------------------- compliance & file integrity
    rule("passwd_file_modified", "Account database modified", "/etc/passwd or /etc/group was written.",
        r#"open.file.path in ["/etc/passwd", "/etc/group", "/etc/gshadow"] && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "TA0003", "T1098", "", "medium"),
    rule("ld_preload_modified", "Dynamic linker preload modified", "/etc/ld.so.preload was written, which makes every program load a library.",
        r#"open.file.path == "/etc/ld.so.preload" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "TA0005", "T1574.006", "", "critical"),
    rule("sshd_config_modified", "SSH server configuration modified", "The SSH daemon's configuration was written.",
        r#"open.file.path =~ "/etc/ssh/sshd_config*" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "", "", "", "low"),
    rule("pam_config_modified", "PAM configuration modified", "A file under /etc/pam.d was written.",
        r#"open.file.path =~ "/etc/pam.d/*" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "TA0006", "T1556.003", "", "medium"),
    rule("systemd_unit_modified", "Systemd unit created or modified", "A systemd unit file was written.",
        r#"open.file.path =~ "/etc/systemd/system/**" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "TA0003", "T1543.002", "", "low"),
    rule("system_binary_modified", "System binary modified", "A program in a system binary directory was written.",
        r#"(open.file.path =~ "/usr/bin/*" || open.file.path =~ "/usr/sbin/*" || open.file.path =~ "/bin/*") && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "", "", "", "medium"),
    rule("hosts_file_modified", "Hosts file modified", "/etc/hosts was written.",
        r#"open.file.path == "/etc/hosts" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0 && container.id == """#, C, "", "", "", "low"),
    rule("kubelet_config_modified", "Kubelet configuration modified", "The kubelet's configuration was written.",
        r#"open.file.path =~ "/var/lib/kubelet/config*" && open.flags & (O_CREAT | O_WRONLY | O_RDWR) > 0"#, C, "", "", "", "medium"),
    // ---------------------------------------------------------------- best practices
    rule("root_process_in_container", "Process started as root in a container", "A container ran a process as uid 0.",
        r#"exec.file.path != "" && container.id != "" && process.uid == 0"#, B, "", "", "", "info"),
    rule("package_installed_on_host", "Package installed on a host", "A package manager changed a host outside of its image or configuration management.",
        r#"exec.file.name in ["apt", "apt-get", "yum", "dnf"] && container.id == "" && process.argv in ["install"]"#, B, "", "", "", "info"),
    rule("sudo_used", "Command run with sudo", "A user ran a command with sudo.",
        r#"exec.file.name == "sudo""#, B, "", "", "", "info"),
];

/// Content packs: (id, name, description).
pub const PACKS: [(&str, &str, &str); 8] = [
    ("container-drift", "Container drift", "Detect containers that change at runtime: shells, package installs and compilers that aren't in the image."),
    ("cryptomining", "Cryptomining", "Detect cryptocurrency miners and connections to mining pools."),
    ("cloud-metadata", "Cloud metadata access", "Detect requests to the instance metadata service that don't use a session token."),
    ("attacker-tools", "Attacker tooling", "Detect reverse shells, socket relays, scanners and file uploads attackers use after gaining access."),
    ("credential-theft", "Credential theft", "Detect reads of password hashes, service account tokens and archived configuration."),
    ("host-persistence", "Host persistence", "Detect cron jobs, SSH keys, accounts and kernel modules attackers add to keep access."),
    ("anti-forensics", "Anti-forensics", "Detect deleted logs and history and flushed firewall rules."),
    ("container-escape", "Container escape", "Detect namespace changes, setuid binaries and process injection used to break out of containers."),
];

/// A `workload_activity` finding rule: a host or container fails it when its agent events matched
/// one of `agent_rules` over the evaluation window.
pub struct ActivityRule {
    pub id: &'static str,
    pub name: &'static str,
    pub severity: &'static str,
    /// host | container
    pub resource_type: &'static str,
    pub agent_rules: &'static [&'static str],
    pub description: &'static str,
}

pub const FINDING_RULES: &[ActivityRule] = &[
    ActivityRule { id: "wp-fr-001", name: "Containers should not execute compilers", severity: "medium", resource_type: "container", agent_rules: &["compiler_in_container"], description: "Build tools belong in the image build, not in running containers." },
    ActivityRule { id: "wp-fr-002", name: "Containers should not install packages at runtime", severity: "medium", resource_type: "container", agent_rules: &["package_manager_in_container"], description: "Packages installed at runtime drift from the scanned image." },
    ActivityRule { id: "wp-fr-003", name: "Instance metadata should only be reached with IMDSv2", severity: "medium", resource_type: "host", agent_rules: &["imds_v1_request"], description: "IMDSv1 requests can be forged through server-side request forgery." },
    ActivityRule { id: "wp-fr-004", name: "Sudoers policy should only change through configuration management", severity: "low", resource_type: "host", agent_rules: &["sudoers_modified"], description: "Manual sudoers changes escape review." },
    ActivityRule { id: "wp-fr-005", name: "SSH authorized keys should not change on hosts", severity: "high", resource_type: "host", agent_rules: &["ssh_authorized_keys_modified"], description: "New keys grant persistent access." },
    ActivityRule { id: "wp-fr-006", name: "Kernel modules should not be loaded at runtime", severity: "high", resource_type: "host", agent_rules: &["kernel_module_loaded"], description: "Modules loaded after boot are a common rootkit technique." },
    ActivityRule { id: "wp-fr-007", name: "User accounts should not be created on hosts", severity: "medium", resource_type: "host", agent_rules: &["user_account_created"], description: "Hosts should get their accounts from the identity provider." },
    ActivityRule { id: "wp-fr-008", name: "Containers should not run processes as root", severity: "low", resource_type: "container", agent_rules: &["root_process_in_container"], description: "A root process makes a container escape more damaging." },
    ActivityRule { id: "wp-fr-009", name: "Critical system files should not be modified", severity: "critical", resource_type: "host", agent_rules: &["passwd_file_modified", "ld_preload_modified"], description: "The account database and the linker preload list change only during provisioning." },
    ActivityRule { id: "wp-fr-010", name: "Shell history should not be cleared", severity: "medium", resource_type: "host", agent_rules: &["shell_history_cleared"], description: "Deleting history hides what was run on the host." },
    ActivityRule { id: "wp-fr-011", name: "Packages should not be installed on hosts by hand", severity: "low", resource_type: "host", agent_rules: &["package_installed_on_host"], description: "Hosts should be rebuilt from images or configuration management." },
];

/// The category Datadog's agent rules report, from the expression's first event type.
pub fn category(expression: &str) -> &'static str {
    let first = expression.trim_start_matches(['(', ' ', '\n', '\t']).split(['.', ' ']).next().unwrap_or("");
    match first {
        "exec" | "fork" | "exit" | "process" | "signal" | "ptrace" | "setuid" | "setgid" | "capset" => "Process Activity",
        "open" | "chmod" | "chown" | "unlink" | "rename" | "mkdir" | "rmdir" | "link" | "utimes" | "setxattr" | "removexattr" | "mount" | "umount" | "splice" => "File Activity",
        "dns" | "bind" | "connect" | "accept" | "imds" | "network" | "packet" => "Network Activity",
        _ => "Kernel Activity",
    }
}

/// Datadog's id form (three groups of three lowercase letters and digits), derived from `seed`
/// so the default rows have the same ids in every org.
pub fn stable_id(seed: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    id_from(h)
}

/// A random id of Datadog's form.
pub fn random_id() -> String {
    id_from(rand::random::<u64>())
}

fn id_from(mut h: u64) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = String::with_capacity(11);
    for i in 0..9 {
        if i > 0 && i % 3 == 0 {
            s.push('-');
        }
        s.push(CHARS[(h % 36) as usize] as char);
        h /= 36;
    }
    s
}

/// The id of a default policy.
pub fn policy_id(key: &str) -> String {
    stable_id(&format!("policy:{key}"))
}

/// The default detection rule raised on an agent rule's events.
pub fn detection_rule_id(agent_rule: &str) -> String {
    format!("def-wp-{}", agent_rule.replace('_', "-"))
}

/// The rule's MITRE tags, as the detection rules and agent rules carry them.
pub fn mitre_tags(r: &AgentRule) -> Vec<String> {
    if r.tactic.is_empty() {
        return vec![];
    }
    let tactic = TACTICS.iter().find(|t| t.0 == r.tactic).map_or("", |t| t.1);
    vec![format!("tactic:{}-{}", r.tactic, tactic.to_lowercase().replace(' ', "-")), format!("technique:{}", r.technique)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_consistent() {
        let mut names: Vec<&str> = AGENT_RULES.iter().map(|r| r.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), AGENT_RULES.len(), "duplicate agent rule names");
        for r in AGENT_RULES {
            assert!(POLICIES.iter().any(|p| p.0 == r.policy), "{}", r.name);
            assert!(r.tactic.is_empty() || TACTICS.iter().any(|t| t.0 == r.tactic), "{}", r.name);
            assert!(r.pack.is_empty() || PACKS.iter().any(|p| p.0 == r.pack), "{}", r.name);
            assert!(["critical", "high", "medium", "low", "info"].contains(&r.severity), "{}", r.name);
        }
        for f in FINDING_RULES {
            assert!(f.agent_rules.iter().all(|a| AGENT_RULES.iter().any(|r| r.name == *a)), "{}", f.id);
        }
    }

    #[test]
    fn ids_have_datadogs_form() {
        let id = stable_id("policy:threat-detection");
        assert_eq!(id, stable_id("policy:threat-detection"));
        assert!(id.len() == 11 && id.chars().enumerate().all(|(i, c)| if i == 3 || i == 7 { c == '-' } else { c.is_ascii_lowercase() || c.is_ascii_digit() }), "{id}");
        assert_ne!(random_id(), random_id());
    }

    #[test]
    fn categories_follow_the_event_type() {
        assert_eq!(category(r#"exec.file.name == "sh""#), "Process Activity");
        assert_eq!(category(r#"(open.file.path == "/etc/sudoers")"#), "File Activity");
        assert_eq!(category(r#"imds.cloud_provider == "aws""#), "Network Activity");
        assert_eq!(category(r#"load_module.name != """#), "Kernel Activity");
    }
}
