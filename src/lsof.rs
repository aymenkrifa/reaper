use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct LsofEntry {
    pub command: String,
    pub pid: String,
    pub user: String,
    pub local_addr: String,
    pub port: u16,
    pub protocol: &'static str,
    pub memory_mb: f64,
    pub start_time: Option<SystemTime>,
}

impl LsofEntry {
    pub fn get_relative_time(&self) -> String {
        match self.start_time {
            Some(start) => start
                .elapsed()
                .map(format_duration)
                .unwrap_or_else(|_| "unknown".to_string()),
            None => "unknown".to_string(),
        }
    }

    pub fn get_memory_display(&self) -> String {
        if self.memory_mb < 1.0 {
            format!("{:.1}KB", self.memory_mb * 1024.0)
        } else if self.memory_mb > 1024.0 {
            format!("{:.1}GB", self.memory_mb / 1024.0)
        } else {
            format!("{:.1}MB", self.memory_mb)
        }
    }

    /// True when we have a real numeric PID we can signal.
    pub fn is_killable(&self) -> bool {
        !self.pid.is_empty() && self.pid.bytes().all(|b| b.is_ascii_digit())
    }
}

pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

const USER_HZ: u64 = 100;
const TCP_LISTEN: &str = "0A";

#[derive(Debug, PartialEq)]
struct Listener {
    local_addr: String,
    port: u16,
    inode: u64,
    uid: u32,
    proto: &'static str,
}

fn parse_proc_net_tcp(content: &str, is_v6: bool) -> Vec<Listener> {
    let mut out = Vec::new();
    for line in content.lines().skip(1) {
        let mut it = line.split_whitespace();
        it.next(); // sl
        let Some(local) = it.next() else { continue };
        it.next(); // rem_address
        let Some(state) = it.next() else { continue };
        if state != TCP_LISTEN {
            continue;
        }
        // tx_queue:rx_queue and tr:tm->when are colon-joined single tokens.
        // Layout after state: tx_queue:rx_queue, tr:tm->when, retrnsmt, uid,
        // timeout, inode.
        it.next(); // tx_queue:rx_queue
        it.next(); // tr:tm->when
        it.next(); // retrnsmt
        let Some(uid_str) = it.next() else { continue };
        let Ok(uid) = uid_str.parse::<u32>() else {
            continue;
        };
        it.next(); // timeout
        let Some(inode_str) = it.next() else { continue };
        let Ok(inode) = inode_str.parse::<u64>() else {
            continue;
        };
        let Some((ip, port)) = parse_hex_addr(local, is_v6) else {
            continue;
        };
        let local_addr = if is_v6 {
            format!("[{}]", ip)
        } else if ip == "0.0.0.0" {
            "*".to_string()
        } else {
            ip
        };
        out.push(Listener {
            local_addr,
            port,
            inode,
            uid,
            proto: if is_v6 { "TCP6" } else { "TCP" },
        });
    }
    out
}

fn parse_hex_addr(s: &str, is_v6: bool) -> Option<(String, u16)> {
    let (addr_hex, port_hex) = s.split_once(':')?;
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    if is_v6 {
        if addr_hex.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = u8::from_str_radix(&addr_hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        // Each 4-byte group is little-endian; reverse within each group.
        for chunk in bytes.chunks_mut(4) {
            chunk.reverse();
        }
        Some((Ipv6Addr::from(bytes).to_string(), port))
    } else {
        if addr_hex.len() != 8 {
            return None;
        }
        let mut bytes = [0u8; 4];
        for i in 0..4 {
            bytes[i] = u8::from_str_radix(&addr_hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        bytes.reverse();
        Some((Ipv4Addr::from(bytes).to_string(), port))
    }
}

fn build_inode_to_pid(needed: &HashSet<u64>) -> HashMap<u64, String> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return map;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else {
            continue;
        };
        if pid_str.is_empty() || !pid_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(target) = fs::read_link(fd.path()) else {
                continue;
            };
            let Some(t) = target.to_str() else { continue };
            if let Some(rest) = t.strip_prefix("socket:[")
                && let Some(num) = rest.strip_suffix(']')
                && let Ok(inode) = num.parse::<u64>()
                && needed.contains(&inode)
            {
                map.entry(inode).or_insert_with(|| pid_str.to_string());
            }
        }
        if map.len() == needed.len() {
            break;
        }
    }
    map
}

fn parse_passwd(content: &str) -> HashMap<u32, String> {
    let mut m = HashMap::new();
    for line in content.lines() {
        let parts: Vec<&str> = line.splitn(7, ':').collect();
        if parts.len() >= 3
            && let Ok(uid) = parts[2].parse::<u32>()
        {
            m.insert(uid, parts[0].to_string());
        }
    }
    m
}

fn passwd_map() -> &'static HashMap<u32, String> {
    static MAP: OnceLock<HashMap<u32, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        fs::read_to_string("/etc/passwd")
            .map(|s| parse_passwd(&s))
            .unwrap_or_default()
    })
}

fn parse_proc_status(content: &str) -> (Option<u32>, f64) {
    let mut uid = None;
    let mut rss_kb: u64 = 0;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            if let Some(first) = rest.split_whitespace().next() {
                uid = first.parse::<u32>().ok();
            }
        } else if let Some(rest) = line.strip_prefix("VmRSS:") {
            rss_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
        }
    }
    (uid, rss_kb as f64 / 1024.0)
}

fn read_proc_status(pid: &str) -> (Option<u32>, f64) {
    fs::read_to_string(format!("/proc/{}/status", pid))
        .map(|c| parse_proc_status(&c))
        .unwrap_or((None, 0.0))
}

fn parse_proc_stat(content: &str, boot_secs: u64) -> (String, Option<SystemTime>) {
    // Field 2 (comm) is parenthesized and may contain spaces or ')'.
    let lparen = content.find('(');
    let rparen = content.rfind(')');
    let (command, after) = match (lparen, rparen) {
        (Some(l), Some(r)) if r > l => (content[l + 1..r].to_string(), &content[r + 1..]),
        _ => return (String::new(), None),
    };
    let fields: Vec<&str> = after.split_whitespace().collect();
    // After comm, field 22 (starttime) sits at index 19.
    let start_time = fields.get(19).and_then(|s| s.parse::<u64>().ok()).map(|t| {
        let start_secs_after_boot = t / USER_HZ;
        let secs_ago = boot_secs.saturating_sub(start_secs_after_boot);
        SystemTime::now() - Duration::from_secs(secs_ago)
    });
    (command, start_time)
}

fn read_proc_stat(pid: &str, boot_secs: u64) -> (String, Option<SystemTime>) {
    fs::read_to_string(format!("/proc/{}/stat", pid))
        .map(|c| parse_proc_stat(&c, boot_secs))
        .unwrap_or((String::new(), None))
}

fn boot_uptime_secs() -> u64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

struct PidMeta {
    command: String,
    user: String,
    memory_mb: f64,
    start_time: Option<SystemTime>,
}

pub fn get_listening_processes() -> Vec<LsofEntry> {
    let tcp = fs::read_to_string("/proc/net/tcp").unwrap_or_default();
    let tcp6 = fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
    let mut listeners = parse_proc_net_tcp(&tcp, false);
    listeners.extend(parse_proc_net_tcp(&tcp6, true));

    let needed: HashSet<u64> = listeners.iter().map(|l| l.inode).collect();
    let inode_to_pid = build_inode_to_pid(&needed);

    let boot = boot_uptime_secs();
    let passwd = passwd_map();
    let mut pid_cache: HashMap<String, PidMeta> = HashMap::new();

    let mut entries = Vec::new();
    for l in listeners {
        let pid = inode_to_pid.get(&l.inode);

        let (command, user, memory_mb, start_time) = match pid {
            Some(pid) => {
                let meta = pid_cache.entry(pid.clone()).or_insert_with(|| {
                    let (uid_opt, memory_mb) = read_proc_status(pid);
                    let user = uid_opt
                        .and_then(|uid| passwd.get(&uid).cloned())
                        .or_else(|| uid_opt.map(|u| u.to_string()))
                        .unwrap_or_else(|| "?".to_string());
                    let (command, start_time) = read_proc_stat(pid, boot);
                    PidMeta {
                        command,
                        user,
                        memory_mb,
                        start_time,
                    }
                });
                (
                    meta.command.clone(),
                    meta.user.clone(),
                    meta.memory_mb,
                    meta.start_time,
                )
            }
            None => {
                // /proc/<pid>/fd was unreadable for every PID we tried (typical
                // when reaper runs without sudo). The /proc/net/tcp row itself
                // still tells us who owns the socket — show that, even though
                // we can't resolve the actual command name.
                let user = passwd
                    .get(&l.uid)
                    .cloned()
                    .unwrap_or_else(|| l.uid.to_string());
                ("(restricted)".to_string(), user, 0.0, None)
            }
        };

        entries.push(LsofEntry {
            command,
            pid: pid.cloned().unwrap_or_else(|| "?".to_string()),
            user,
            local_addr: l.local_addr,
            port: l.port,
            protocol: l.proto,
            memory_mb,
            start_time,
        });
    }

    entries
}

/// Outcome of attempting to terminate a process.
#[derive(Debug)]
pub enum KillOutcome {
    /// Process terminated within the verification window.
    Terminated,
    /// SIGTERM was ignored; SIGKILL got it.
    ForceKilled,
    /// Process is still alive even after SIGKILL.
    StillAlive,
}

/// Send SIGTERM, wait up to ~200ms for the process to exit, escalate to
/// SIGKILL if needed, then wait another ~200ms. Returns what actually
/// happened — no lying about "successfully killed" when we only sent a
/// signal.
pub fn kill_process_verified(pid: &str) -> io::Result<KillOutcome> {
    send_signal(pid, "-TERM")?;
    if wait_for_exit(pid, Duration::from_millis(200)) {
        return Ok(KillOutcome::Terminated);
    }
    send_signal(pid, "-KILL")?;
    if wait_for_exit(pid, Duration::from_millis(200)) {
        return Ok(KillOutcome::ForceKilled);
    }
    Ok(KillOutcome::StillAlive)
}

fn wait_for_exit(pid: &str, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    let step = Duration::from_millis(20);
    loop {
        if fs::metadata(format!("/proc/{}", pid)).is_err() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(step);
    }
}

fn send_signal(pid: &str, sig: &str) -> io::Result<()> {
    if !pid.bytes().all(|b| b.is_ascii_digit()) || pid.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid pid: {}", pid),
        ));
    }
    let output = Command::new("kill").arg(sig).arg(pid).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(io::Error::other(format!(
            "kill {} {} failed: {}",
            sig, pid, stderr
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ipv4_hex_addr() {
        // 127.0.0.1:8080 (kernel formats addr as LE u32 hex)
        assert_eq!(
            parse_hex_addr("0100007F:1F90", false),
            Some(("127.0.0.1".to_string(), 8080))
        );
        // 0.0.0.0:80
        assert_eq!(
            parse_hex_addr("00000000:0050", false),
            Some(("0.0.0.0".to_string(), 80))
        );
        // 192.168.1.1:443
        assert_eq!(
            parse_hex_addr("0101A8C0:01BB", false),
            Some(("192.168.1.1".to_string(), 443))
        );
    }

    #[test]
    fn parse_ipv6_hex_addr() {
        // :: with port 8080
        assert_eq!(
            parse_hex_addr("00000000000000000000000000000000:1F90", true),
            Some(("::".to_string(), 8080))
        );
        // ::1 with port 80
        assert_eq!(
            parse_hex_addr("00000000000000000000000001000000:0050", true),
            Some(("::1".to_string(), 80))
        );
        // 2001:db8:: with port 80 — first 4-byte group is LE per the kernel format
        assert_eq!(
            parse_hex_addr("B80D0120000000000000000000000000:0050", true),
            Some(("2001:db8::".to_string(), 80))
        );
    }

    #[test]
    fn rejects_malformed_hex_addr() {
        assert_eq!(parse_hex_addr("ZZ:80", false), None); // bad hex
        assert_eq!(parse_hex_addr("0100007F", false), None); // no port
        assert_eq!(parse_hex_addr("01:1F90", false), None); // wrong length v4
        assert_eq!(parse_hex_addr("01:1F90", true), None); // wrong length v6
    }

    #[test]
    fn parse_proc_net_tcp_extracts_listeners_only() {
        // Header line + one LISTEN, one ESTABLISHED, one LISTEN.
        let content = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 318513 2 0000000000000000
   1: 0100007F:0050 0100007F:ABCD 01 00000000:00000000 00:00000000 00000000  1000        0 999999 1 0000000000000000
   2: 00000000:01BB 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 24707  1 0000000000000000
";
        let listeners = parse_proc_net_tcp(content, false);
        assert_eq!(listeners.len(), 2);
        assert_eq!(listeners[0].port, 8080);
        assert_eq!(listeners[0].local_addr, "127.0.0.1");
        assert_eq!(listeners[0].inode, 318513);
        assert_eq!(listeners[0].uid, 1000);
        assert_eq!(listeners[0].proto, "TCP");
        assert_eq!(listeners[1].port, 443);
        assert_eq!(listeners[1].local_addr, "*");
        assert_eq!(listeners[1].inode, 24707);
        assert_eq!(listeners[1].uid, 0);
    }

    #[test]
    fn parse_proc_net_tcp6_formats_v6() {
        let content = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:280A 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 26190 1
";
        let listeners = parse_proc_net_tcp(content, true);
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].local_addr, "[::]");
        assert_eq!(listeners[0].port, 0x280A);
        assert_eq!(listeners[0].proto, "TCP6");
    }

    #[test]
    fn parse_proc_stat_handles_parens_in_comm() {
        // Process names can contain spaces and ')' — only the LAST ')' delimits comm.
        // Field 22 (starttime) is at index 19 after the rparen.
        // We construct a stat line with comm = "weird (name)" and starttime = 100.
        let mut fields = vec![
            "S".to_string(), "1".to_string(), // state, ppid (idx 0,1)
        ];
        for i in 2..19 {
            fields.push(i.to_string());
        }
        fields.push("100".to_string()); // starttime at index 19
        for _ in 20..30 {
            fields.push("0".to_string());
        }
        let content = format!("123 (weird (name)) {}", fields.join(" "));
        // boot_secs = 1000, USER_HZ = 100 → start was at 100/100 = 1s after boot
        // → secs_ago = 1000 - 1 = 999
        let (cmd, start) = parse_proc_stat(&content, 1000);
        assert_eq!(cmd, "weird (name)");
        let elapsed = SystemTime::now()
            .duration_since(start.unwrap())
            .unwrap()
            .as_secs();
        assert!(
            (995..=1005).contains(&elapsed),
            "expected ~999s, got {}",
            elapsed
        );
    }

    #[test]
    fn parse_proc_status_extracts_uid_and_rss() {
        let content = "\
Name:\tnginx
Uid:\t33\t33\t33\t33
VmRSS:\t  4096 kB
";
        let (uid, rss_mb) = parse_proc_status(content);
        assert_eq!(uid, Some(33));
        assert!((rss_mb - 4.0).abs() < 1e-9);
    }

    #[test]
    fn parse_passwd_basic() {
        let content = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
www-data:x:33:33:www-data:/var/www:/usr/sbin/nologin
";
        let m = parse_passwd(content);
        assert_eq!(m.get(&0), Some(&"root".to_string()));
        assert_eq!(m.get(&33), Some(&"www-data".to_string()));
        assert_eq!(m.get(&1), Some(&"daemon".to_string()));
    }

    #[test]
    fn format_duration_units() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59m");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(86_399)), "23h");
        assert_eq!(format_duration(Duration::from_secs(86_400)), "1d");
        assert_eq!(format_duration(Duration::from_secs(86_400 * 30)), "30d");
    }

    #[test]
    fn lsof_entry_killable_check() {
        let mut e = LsofEntry {
            command: "x".into(),
            pid: "1234".into(),
            user: "u".into(),
            local_addr: "*".into(),
            port: 80,
            protocol: "TCP",
            memory_mb: 0.0,
            start_time: None,
        };
        assert!(e.is_killable());
        e.pid = "?".into();
        assert!(!e.is_killable());
        e.pid = "".into();
        assert!(!e.is_killable());
        e.pid = "12a4".into();
        assert!(!e.is_killable());
    }
}
