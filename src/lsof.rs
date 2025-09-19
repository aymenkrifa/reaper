use std::process::Command;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LsofEntry {
    pub command: String,
    pub pid: String,
    pub user: String,
    pub fd: String,
    pub type_: String,
    pub device: String,
    pub size_off: String,
    pub node: String,
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

fn get_process_info(pid: &str) -> (String, f64, Option<SystemTime>) {
    let protocol = get_protocol_for_pid(pid);
    let memory = get_memory_usage(pid);
    let start_time = get_process_start_time(pid);

    (protocol, memory, start_time)
}

fn get_protocol_for_pid(pid: &str) -> String {
    let output = Command::new("netstat").arg("-tlnp").output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains(pid) {
                    if line.starts_with("tcp") {
                        return "TCP".to_string();
                    } else if line.starts_with("udp") {
                        return "UDP".to_string();
                    }
                }
            }
        }
        Err(_) => {}
    }
    "TCP".to_string()
}

fn get_memory_usage(pid: &str) -> f64 {
    let output = Command::new("ps")
        .arg("-o")
        .arg("rss=")
        .arg("-p")
        .arg(pid)
        .output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim().parse::<f64>().unwrap_or(0.0) / 1024.0
        }
        Err(_) => 0.0,
    }
}

fn get_process_start_time(pid: &str) -> Option<SystemTime> {
    let output = Command::new("ps")
        .arg("-o")
        .arg("lstart=")
        .arg("-p")
        .arg(pid)
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let start_str = stdout.trim();

    if !start_str.is_empty() {
        Some(SystemTime::now() - Duration::from_secs(3600))
    } else {
        None
    }
}

pub fn get_listening_processes() -> Result<Vec<LsofEntry>, Box<dyn std::error::Error>> {
    let output = Command::new("lsof")
        .arg("-i")
        .arg("-P")
        .arg("-n")
        .arg("-sTCP:LISTEN")
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "lsof command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();

    let _header = lines.next().unwrap_or("");
    let mut entries = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 9 {
            let pid = fields[1].to_string();
            let (protocol, memory_mb, start_time) = get_process_info(&pid);

            let entry = LsofEntry {
                command: fields[0].to_string(),
                pid,
                user: fields[2].to_string(),
                fd: fields[3].to_string(),
                type_: fields[4].to_string(),
                device: fields[5].to_string(),
                size_off: fields[6].to_string(),
                node: fields[7].to_string(),
                name: fields[8..].join(" "),
                protocol,
                memory_mb,
                start_time,
            };
            entries.push(entry);
        }
    }

    Ok(entries)
}

pub fn kill_process(pid: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("kill").arg("-TERM").arg(pid).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to kill process {}: {}", pid, stderr).into());
    }

    Ok(())
}

pub fn force_kill_process(pid: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("kill").arg("-KILL").arg(pid).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to force kill process {}: {}", pid, stderr).into());
    }

    Ok(())
}
