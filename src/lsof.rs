use std::process::Command;

#[derive(Debug, Clone)]
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
}

pub fn get_listening_processes() -> Result<Vec<LsofEntry>, Box<dyn std::error::Error>> {
    let output = Command::new("lsof")
        .arg("-i")
        .arg("-P")
        .arg("-n")
        .arg("-sTCP:LISTEN")
        .output()?;

    if !output.status.success() {
        return Err(format!("lsof command failed: {}", String::from_utf8_lossy(&output.stderr)).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();

    // Skip header
    let _header = lines.next().unwrap_or("");
    let mut entries = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 9 {
            let entry = LsofEntry {
                command: fields[0].to_string(),
                pid: fields[1].to_string(),
                user: fields[2].to_string(),
                fd: fields[3].to_string(),
                type_: fields[4].to_string(),
                device: fields[5].to_string(),
                size_off: fields[6].to_string(),
                node: fields[7].to_string(),
                name: fields[8..].join(" "),
            };
            entries.push(entry);
        }
    }

    Ok(entries)
}