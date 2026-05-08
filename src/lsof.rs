use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct LsofEntry {
    pub command: String,
    pub pid: String,
    pub user: String,
    pub name: String,
    pub protocol: String,
    pub memory_mb: f64,
    pub start_time: Option<SystemTime>,
}

impl LsofEntry {
    pub fn get_relative_time(&self) -> String {
        match self.start_time {
            Some(start) => {
                if let Ok(elapsed) = start.elapsed() {
                    format_duration(elapsed)
                } else {
                    "unknown".to_string()
                }
            }
            None => "unknown".to_string(),
        }
    }

    pub fn get_protocol(&self) -> &str {
        &self.protocol
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
}

fn format_duration(duration: Duration) -> String {
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

#[derive(Debug)]
struct Listener {
    display: String,
    inode: u64,
    proto: &'static str,
}

fn parse_proc_net_tcp(path: &str, is_v6: bool) -> Vec<Listener> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
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
        // The kernel formats tx_queue:rx_queue and tr:tm->when as single
        // colon-joined tokens, so only 5 fields sit between state and inode:
        // tx_queue:rx_queue, tr:tm->when, retrnsmt, uid, timeout.
        for _ in 0..5 {
            it.next();
        }
        let Some(inode_str) = it.next() else { continue };
        let Ok(inode) = inode_str.parse::<u64>() else {
            continue;
        };
        let Some((ip, port)) = parse_hex_addr(local, is_v6) else {
            continue;
        };
        let display = if is_v6 {
            format!("[{}]:{} (LISTEN)", ip, port)
        } else if ip == "0.0.0.0" {
            format!("*:{} (LISTEN)", port)
        } else {
            format!("{}:{} (LISTEN)", ip, port)
        };
        out.push(Listener {
            display,
            inode,
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

fn passwd_map() -> &'static HashMap<u32, String> {
    static MAP: OnceLock<HashMap<u32, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        if let Ok(content) = fs::read_to_string("/etc/passwd") {
            for line in content.lines() {
                let parts: Vec<&str> = line.splitn(7, ':').collect();
                if parts.len() >= 3
                    && let Ok(uid) = parts[2].parse::<u32>()
                {
                    m.insert(uid, parts[0].to_string());
                }
            }
        }
        m
    })
}

fn read_proc_status(pid: &str) -> (Option<u32>, f64) {
    let Ok(content) = fs::read_to_string(format!("/proc/{}/status", pid)) else {
        return (None, 0.0);
    };
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

fn read_proc_stat(pid: &str, boot_secs: u64) -> (String, Option<SystemTime>) {
    let Ok(content) = fs::read_to_string(format!("/proc/{}/stat", pid)) else {
        return (String::new(), None);
    };
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

pub fn get_listening_processes() -> Result<Vec<LsofEntry>, Box<dyn std::error::Error>> {
    let mut listeners = parse_proc_net_tcp("/proc/net/tcp", false);
    listeners.extend(parse_proc_net_tcp("/proc/net/tcp6", true));

    let needed: HashSet<u64> = listeners.iter().map(|l| l.inode).collect();
    let inode_to_pid = build_inode_to_pid(&needed);

    let boot = boot_uptime_secs();
    let passwd = passwd_map();
    let mut pid_cache: HashMap<String, PidMeta> = HashMap::new();

    let mut entries = Vec::new();
    for l in listeners {
        let Some(pid) = inode_to_pid.get(&l.inode) else {
            continue;
        };
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

        entries.push(LsofEntry {
            command: meta.command.clone(),
            pid: pid.clone(),
            user: meta.user.clone(),
            name: l.display,
            protocol: l.proto.to_string(),
            memory_mb: meta.memory_mb,
            start_time: meta.start_time,
        });
    }

    Ok(entries)
}

pub fn kill_process(pid: &str) -> Result<(), Box<dyn std::error::Error>> {
    send_signal(pid, "-TERM")
}

pub fn force_kill_process(pid: &str) -> Result<(), Box<dyn std::error::Error>> {
    send_signal(pid, "-KILL")
}

fn send_signal(pid: &str, sig: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !pid.bytes().all(|b| b.is_ascii_digit()) || pid.is_empty() {
        return Err(format!("invalid pid: {}", pid).into());
    }
    let output = Command::new("kill").arg(sig).arg(pid).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("kill {} {} failed: {}", sig, pid, stderr).into());
    }
    Ok(())
}
