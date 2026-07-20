use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
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
    /// Raw starttime from /proc/<pid>/stat (clock ticks since boot).
    /// Identifies a specific incarnation of a PID: if the kernel recycles
    /// the number for a new process, its ticks differ. Used to refuse
    /// killing a recycled PID.
    pub starttime_ticks: Option<u64>,
    /// Working directory the process was started in, when readable.
    /// `None` for restricted PIDs (other users) or kernel threads.
    pub cwd: Option<String>,
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

    /// True when we have a real numeric PID we can signal — the same
    /// definition send_signal enforces, so a row the UI offers to kill
    /// can't be rejected later as an invalid pid.
    pub fn is_killable(&self) -> bool {
        self.pid.parse::<i32>().is_ok_and(|p| p > 0)
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

#[derive(Debug)]
struct ProcStat {
    comm: String,
    /// Single-char process state (field 3): 'R', 'S', 'Z', …
    state: Option<char>,
    /// Field 22: process start time in clock ticks since boot.
    starttime_ticks: Option<u64>,
}

fn parse_proc_stat(content: &str) -> Option<ProcStat> {
    // Field 2 (comm) is parenthesized and may contain spaces or ')' —
    // only the LAST ')' delimits it.
    let lparen = content.find('(')?;
    let rparen = content.rfind(')')?;
    if rparen <= lparen {
        return None;
    }
    let comm = content[lparen + 1..rparen].to_string();
    let fields: Vec<&str> = content[rparen + 1..].split_whitespace().collect();
    let state = fields.first().and_then(|s| s.chars().next());
    // After comm, field 22 (starttime) sits at index 19.
    let starttime_ticks = fields.get(19).and_then(|s| s.parse::<u64>().ok());
    Some(ProcStat {
        comm,
        state,
        starttime_ticks,
    })
}

fn read_proc_stat(pid: &str) -> Option<ProcStat> {
    fs::read_to_string(format!("/proc/{}/stat", pid))
        .ok()
        .and_then(|c| parse_proc_stat(&c))
}

fn start_time_from_ticks(ticks: u64, uptime_secs: u64) -> SystemTime {
    let start_secs_after_boot = ticks / USER_HZ;
    let secs_ago = uptime_secs.saturating_sub(start_secs_after_boot);
    SystemTime::now() - Duration::from_secs(secs_ago)
}

/// Interpreter-style executables whose first argument is a script path
/// rather than a flag — so basenaming that path collapses noise like
/// `/home/user/testing/k3s-inspector` to `k3s-inspector` without losing
/// meaning.
const INTERPRETERS: &[&str] = &[
    "python", "node", "ruby", "perl", "java", "sh", "bash", "zsh", "fish", "dash", "deno", "bun",
    "lua", "tcl", "php", "pwsh",
];

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// Strip trailing version digits/dots so `python3.11`, `python3`, and
/// `node22` all match the bare interpreter name.
fn strip_version(name: &str) -> &str {
    name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')
}

/// Build a human-readable command string from /proc/<pid>/cmdline.
///
/// /proc/<pid>/cmdline is the raw argv with NUL separators. arg0 is
/// always basenamed so the line starts with the program name (`nginx`)
/// rather than its install path (`/usr/sbin/nginx`).
///
/// When arg0 is a known interpreter (python, node, ruby, …) we also
/// basename arg1, because for interpreters arg1 *is* the script. Without
/// this, every Python listener reads
/// `python /home/user/testing/k3s-inspector` — accurate but hard to scan.
/// With it, the same listener reads `python k3s-inspector` and the
/// project name pops. Args after the script are kept verbatim — those
/// are flags and values the user actually typed.
pub(crate) fn parse_cmdline(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let s = String::from_utf8_lossy(raw);
    let parts: Vec<&str> = s.split('\0').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let arg0 = basename(parts[0]);
    let is_interp = INTERPRETERS.contains(&strip_version(arg0));

    if parts.len() == 1 {
        Some(arg0.to_string())
    } else if is_interp {
        let script = basename(parts[1]);
        if parts.len() == 2 {
            Some(format!("{} {}", arg0, script))
        } else {
            Some(format!("{} {} {}", arg0, script, parts[2..].join(" ")))
        }
    } else {
        Some(format!("{} {}", arg0, parts[1..].join(" ")))
    }
}

fn read_proc_cmdline(pid: &str) -> Option<String> {
    let raw = fs::read(format!("/proc/{}/cmdline", pid)).ok()?;
    parse_cmdline(&raw)
}

/// Resolve /proc/<pid>/cwd, the symlink to the process's working directory.
/// Returns `None` for restricted PIDs (the symlink is unreadable for other
/// users without CAP_SYS_PTRACE) and kernel threads.
fn read_proc_cwd(pid: &str) -> Option<String> {
    fs::read_link(format!("/proc/{}/cwd", pid))
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
}

fn boot_uptime_secs() -> u64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

/// Everything we display about one PID, read fresh each scan. Processes
/// rewrite their argv (setproctitle), drop privileges (setuid), and
/// chdir at runtime, so none of this can be cached across scans without
/// going stale — and at a handful of small /proc reads per listener per
/// second, it doesn't need to be.
#[derive(Debug)]
struct PidMeta {
    starttime_ticks: Option<u64>,
    command: String,
    user: String,
    memory_mb: f64,
    start_time: Option<SystemTime>,
    cwd: Option<String>,
}

/// Carries the socket-inode → PID mapping across scans.
///
/// Resolving an inode to its owning PID means walking every fd of every
/// process under /proc — by far the most expensive part of a scan. The
/// mapping is cached so that walk only runs when an unknown inode
/// appears. Each entry records the starttime ticks of the PID it was
/// resolved against; a scan that observes different ticks (the PID died,
/// possibly recycled for an unrelated process) evicts the entry and
/// re-resolves, so a recycled PID can never inherit the old socket's
/// attribution.
#[derive(Debug, Default)]
pub struct Scanner {
    inode_to_pid: HashMap<u64, (String, Option<u64>)>,
}

impl Scanner {
    pub fn scan(&mut self) -> Vec<LsofEntry> {
        let tcp = fs::read_to_string("/proc/net/tcp").unwrap_or_default();
        let tcp6 = fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
        let mut listeners = parse_proc_net_tcp(&tcp, false);
        listeners.extend(parse_proc_net_tcp(&tcp6, true));

        let needed: HashSet<u64> = listeners.iter().map(|l| l.inode).collect();
        self.inode_to_pid.retain(|inode, _| needed.contains(inode));

        // One stat read per distinct PID per scan — the identity check
        // here, comm/start-time for the entries below.
        let mut stats: HashMap<String, Option<ProcStat>> = HashMap::new();
        for (pid, _) in self.inode_to_pid.values() {
            stats
                .entry(pid.clone())
                .or_insert_with(|| read_proc_stat(pid));
        }
        // Keep a mapping only while its PID is provably the same
        // incarnation it was resolved against.
        self.inode_to_pid.retain(|_, (pid, ticks)| {
            let live = stats
                .get(pid)
                .and_then(|s| s.as_ref())
                .and_then(|s| s.starttime_ticks);
            matches!((live, *ticks), (Some(l), Some(t)) if l == t)
        });

        let unknown: HashSet<u64> = needed
            .iter()
            .copied()
            .filter(|inode| !self.inode_to_pid.contains_key(inode))
            .collect();
        if !unknown.is_empty() {
            for (inode, pid) in build_inode_to_pid(&unknown) {
                let stat = stats
                    .entry(pid.clone())
                    .or_insert_with(|| read_proc_stat(&pid));
                let ticks = stat.as_ref().and_then(|s| s.starttime_ticks);
                self.inode_to_pid.insert(inode, (pid, ticks));
            }
        }

        let uptime = boot_uptime_secs();
        let passwd = passwd_map();
        // Scan-local, so a PID backing several listeners (v4+v6) is only
        // read once per scan.
        let mut pid_cache: HashMap<String, PidMeta> = HashMap::new();

        let mut entries = Vec::new();
        for l in listeners {
            let pid = self.inode_to_pid.get(&l.inode).map(|(p, _)| p.clone());

            let entry = match pid {
                Some(pid) => {
                    let meta = pid_cache.entry(pid.clone()).or_insert_with(|| {
                        let stat = stats.remove(&pid).flatten();
                        let ticks = stat.as_ref().and_then(|s| s.starttime_ticks);
                        let comm = stat.map(|s| s.comm).unwrap_or_default();
                        let (uid_opt, memory_mb) = read_proc_status(&pid);
                        let user = uid_opt
                            .map(|uid| resolve_user(uid, passwd))
                            .unwrap_or_else(|| "?".to_string());
                        // Prefer the full argv from /proc/<pid>/cmdline so we
                        // can distinguish two `python3` / `uvicorn` listeners
                        // by the script or app they're running. Falls back to
                        // the 15-char comm name when cmdline is empty (kernel
                        // threads, zombies, races).
                        let command = read_proc_cmdline(&pid).unwrap_or(comm);
                        let cwd = read_proc_cwd(&pid);
                        let start_time = ticks.map(|t| start_time_from_ticks(t, uptime));
                        PidMeta {
                            starttime_ticks: ticks,
                            command,
                            user,
                            memory_mb,
                            start_time,
                            cwd,
                        }
                    });
                    LsofEntry {
                        command: meta.command.clone(),
                        pid: pid.clone(),
                        user: meta.user.clone(),
                        local_addr: l.local_addr,
                        port: l.port,
                        protocol: l.proto,
                        memory_mb: meta.memory_mb,
                        start_time: meta.start_time,
                        starttime_ticks: meta.starttime_ticks,
                        cwd: meta.cwd.clone(),
                    }
                }
                None => {
                    // /proc/<pid>/fd was unreadable for every PID we tried
                    // (typical when reaper runs without sudo). The
                    // /proc/net/tcp row itself still tells us who owns the
                    // socket — show that, even though we can't resolve the
                    // actual command name.
                    LsofEntry {
                        command: "(restricted)".to_string(),
                        pid: "?".to_string(),
                        user: resolve_user(l.uid, passwd),
                        local_addr: l.local_addr,
                        port: l.port,
                        protocol: l.proto,
                        memory_mb: 0.0,
                        start_time: None,
                        starttime_ticks: None,
                        cwd: None,
                    }
                }
            };
            entries.push(entry);
        }

        entries
    }
}

/// uid → username via /etc/passwd, falling back to the numeric uid.
fn resolve_user(uid: u32, passwd: &HashMap<u32, String>) -> String {
    passwd.get(&uid).cloned().unwrap_or_else(|| uid.to_string())
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
///
/// `expected_ticks` is the starttime of the process the user confirmed
/// killing. The confirmation prompt has no timeout, so by the time we get
/// here the kernel may have recycled the PID for an unrelated process —
/// refuse to signal anything we can't positively identify. A snapshot
/// without ticks (a /proc read race at scan time) also refuses: failing
/// closed beats signaling an unverified PID.
pub fn kill_process_verified(pid: &str, expected_ticks: Option<u64>) -> io::Result<KillOutcome> {
    let Some(expected) = expected_ticks else {
        return Err(io::Error::other(
            "could not verify process identity — refresh (r) and retry",
        ));
    };
    match read_proc_stat(pid).and_then(|s| s.starttime_ticks) {
        // Already gone — the goal state, nothing to signal.
        None => return Ok(KillOutcome::Terminated),
        Some(t) if t != expected => {
            return Err(io::Error::other(
                "PID was recycled by a different process; not killing",
            ));
        }
        Some(_) => {}
    }
    send_signal(pid, libc::SIGTERM)?;
    if wait_for_exit(pid, Duration::from_millis(200)) {
        return Ok(KillOutcome::Terminated);
    }
    send_signal(pid, libc::SIGKILL)?;
    if wait_for_exit(pid, Duration::from_millis(200)) {
        return Ok(KillOutcome::ForceKilled);
    }
    Ok(KillOutcome::StillAlive)
}

/// A process counts as exited once its /proc entry is gone or it has
/// turned into a zombie ('Z') or dead ('X') task — a zombie keeps its
/// /proc entry until the parent reaps it, but its sockets are already
/// released.
fn has_exited(pid: &str) -> bool {
    match read_proc_stat(pid) {
        None => true,
        Some(stat) => matches!(stat.state, Some('Z') | Some('X') | None),
    }
}

fn wait_for_exit(pid: &str, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    let step = Duration::from_millis(20);
    loop {
        if has_exited(pid) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(step);
    }
}

fn send_signal(pid: &str, sig: libc::c_int) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, format!("invalid pid: {}", pid));
    let pid_num: i32 = pid.parse().map_err(|_| invalid())?;
    // pid 0 / negative values signal entire process groups — never that.
    if pid_num <= 0 {
        return Err(invalid());
    }
    if unsafe { libc::kill(pid_num, sig) } == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        // Process disappeared on its own — the goal state.
        return Ok(());
    }
    Err(err)
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

    /// Build a synthetic /proc/<pid>/stat line with the given comm, state
    /// and starttime (field 22, index 19 after comm).
    fn stat_line(comm: &str, state: &str, starttime: &str) -> String {
        let mut fields = vec![state.to_string(), "1".to_string()]; // state, ppid
        for i in 2..19 {
            fields.push(i.to_string());
        }
        fields.push(starttime.to_string()); // starttime at index 19
        for _ in 20..30 {
            fields.push("0".to_string());
        }
        format!("123 ({}) {}", comm, fields.join(" "))
    }

    #[test]
    fn parse_proc_stat_handles_parens_in_comm() {
        // Process names can contain spaces and ')' — only the LAST ')' delimits comm.
        let content = stat_line("weird (name)", "S", "100");
        let stat = parse_proc_stat(&content).unwrap();
        assert_eq!(stat.comm, "weird (name)");
        assert_eq!(stat.state, Some('S'));
        assert_eq!(stat.starttime_ticks, Some(100));

        // uptime = 1000s, USER_HZ = 100 → start was at 100/100 = 1s after
        // boot → secs_ago = 1000 - 1 = 999
        let start = start_time_from_ticks(100, 1000);
        let elapsed = SystemTime::now().duration_since(start).unwrap().as_secs();
        assert!(
            (995..=1005).contains(&elapsed),
            "expected ~999s, got {}",
            elapsed
        );
    }

    #[test]
    fn parse_proc_stat_extracts_zombie_state() {
        let stat = parse_proc_stat(&stat_line("dead-server", "Z", "42")).unwrap();
        assert_eq!(stat.state, Some('Z'));
        assert_eq!(stat.starttime_ticks, Some(42));
    }

    #[test]
    fn send_signal_rejects_non_positive_and_malformed_pids() {
        // pid 0 / negatives would signal whole process groups.
        for pid in ["0", "-1", "", "abc", "12a4"] {
            let err = send_signal(pid, libc::SIGTERM).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "pid {:?}", pid);
        }
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
    fn parse_cmdline_basenames_python_script() {
        // The wrapper-script case: argv is python + path-to-uvicorn-shim + app.
        let raw = b"/usr/bin/python3\0/home/u/.venv/bin/uvicorn\0app.main:app\0--port\0:8000\0";
        assert_eq!(
            parse_cmdline(raw).as_deref(),
            Some("python3 uvicorn app.main:app --port :8000")
        );
    }

    #[test]
    fn parse_cmdline_basenames_user_script_path() {
        // The k3s-inspector case the user reported.
        let raw = b"/usr/bin/python\0/home/user/testing/k3s-inspector\0";
        assert_eq!(parse_cmdline(raw).as_deref(), Some("python k3s-inspector"));
    }

    #[test]
    fn parse_cmdline_keeps_flags_for_non_interpreter() {
        // For a non-interpreter binary, args after arg0 are flags/values
        // the user typed — don't basename them.
        let raw = b"/usr/sbin/nginx\0-c\0/etc/nginx.conf\0";
        assert_eq!(
            parse_cmdline(raw).as_deref(),
            Some("nginx -c /etc/nginx.conf")
        );
    }

    #[test]
    fn parse_cmdline_handles_versioned_interpreter() {
        // python3.11 / node22 should be recognized as interpreters.
        let raw = b"/usr/bin/python3.11\0/path/script.py\0";
        assert_eq!(parse_cmdline(raw).as_deref(), Some("python3.11 script.py"));
        let raw = b"/usr/bin/node22\0/path/server.js\0";
        assert_eq!(parse_cmdline(raw).as_deref(), Some("node22 server.js"));
    }

    #[test]
    fn parse_cmdline_single_arg_no_path() {
        let raw = b"sshd\0";
        assert_eq!(parse_cmdline(raw).as_deref(), Some("sshd"));
    }

    #[test]
    fn parse_cmdline_with_full_path_only() {
        let raw = b"/usr/sbin/nginx\0";
        assert_eq!(parse_cmdline(raw).as_deref(), Some("nginx"));
    }

    #[test]
    fn parse_cmdline_empty_returns_none() {
        // Kernel threads and zombies have empty cmdline.
        assert_eq!(parse_cmdline(b""), None);
        assert_eq!(parse_cmdline(b"\0\0"), None);
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
            starttime_ticks: None,
            cwd: None,
        };
        assert!(e.is_killable());
        e.pid = "?".into();
        assert!(!e.is_killable());
        e.pid = "".into();
        assert!(!e.is_killable());
        e.pid = "12a4".into();
        assert!(!e.is_killable());
        // Aligned with send_signal: no pid 0, nothing beyond i32.
        e.pid = "0".into();
        assert!(!e.is_killable());
        e.pid = "9999999999".into();
        assert!(!e.is_killable());
    }

    #[test]
    fn kill_refuses_unverifiable_identity() {
        // A snapshot whose starttime couldn't be captured at scan time
        // must fail closed rather than signal an unverified PID.
        let err = kill_process_verified("1", None).unwrap_err();
        assert!(err.to_string().contains("identity"), "got: {}", err);
    }
}
