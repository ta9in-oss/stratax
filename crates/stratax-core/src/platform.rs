//! OS-specific probes. Only Linux is implemented; other systems get a degraded
//! but working `Generic` backend built on `sysinfo`.

use crate::model::DiskInfo;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProcPaths {
    pub pid: u32,
    pub name: String,
    pub cwd: Option<PathBuf>,
    /// Directories / files the process holds open (Linux only).
    pub open: Vec<PathBuf>,
}

pub trait Platform: Send + Sync {
    fn disks(&self) -> Vec<DiskInfo>;
    /// cwd + open paths of every process we are allowed to inspect.
    fn process_paths(&self, with_fds: bool) -> Vec<ProcPaths>;
    fn threads_of(&self, pid: u32) -> Option<u32>;
    fn temp_dirs(&self) -> Vec<PathBuf> {
        vec![std::env::temp_dir()]
    }
}

pub fn current() -> Box<dyn Platform> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::Linux)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(Generic)
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Drop the calling thread to nice 10 and the lowest best-effort I/O level.
/// (The idle class looks nicer on paper but starves completely while agents
/// keep the disk busy compiling — a scan then takes many minutes.)
pub fn lower_priority() {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
        #[cfg(target_os = "linux")]
        {
            const IOPRIO_WHO_PROCESS: libc::c_long = 1;
            const IOPRIO_CLASS_BE: libc::c_long = 2;
            libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0 as libc::c_long, (IOPRIO_CLASS_BE << 13) | 7);
        }
    }
}

pub fn statvfs(path: &Path) -> Option<(u64, u64, u64)> {
    use std::ffi::CString;
    let c = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = s.f_frsize as u64;
    let total = s.f_blocks as u64 * f;
    let avail = s.f_bavail as u64 * f;
    let free = s.f_bfree as u64 * f;
    Some((total, total.saturating_sub(free), avail))
}

#[allow(dead_code)]
pub struct Generic;

impl Platform for Generic {
    fn disks(&self) -> Vec<DiskInfo> {
        sysinfo::Disks::new_with_refreshed_list()
            .iter()
            .map(|d| DiskInfo {
                name: d.name().to_string_lossy().into_owned(),
                device: d.name().to_string_lossy().into_owned(),
                label: None,
                fs: Some(d.file_system().to_string_lossy().into_owned()),
                mount: Some(d.mount_point().to_path_buf()),
                total: d.total_space(),
                used: d.total_space().saturating_sub(d.available_space()),
                available: d.available_space(),
                rotational: matches!(d.kind(), sysinfo::DiskKind::HDD),
                removable: d.is_removable(),
                uuid: None,
                in_fstab: true,
            })
            .collect()
    }
    fn process_paths(&self, _with_fds: bool) -> Vec<ProcPaths> {
        let mut s = sysinfo::System::new();
        s.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        s.processes()
            .iter()
            .map(|(pid, p)| ProcPaths {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cwd: p.cwd().map(Path::to_path_buf),
                open: vec![],
            })
            .collect()
    }
    fn threads_of(&self, _pid: u32) -> Option<u32> {
        None
    }
}

#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;
    use std::fs;

    pub struct Linux;

    impl Platform for Linux {
        fn disks(&self) -> Vec<DiskInfo> {
            let mut out = lsblk().unwrap_or_default();
            if out.is_empty() {
                out = Generic.disks();
            }
            out
        }

        fn process_paths(&self, with_fds: bool) -> Vec<ProcPaths> {
            let Ok(rd) = fs::read_dir("/proc") else { return vec![] };
            rd.flatten()
                .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
                .filter_map(|pid| {
                    let base = PathBuf::from(format!("/proc/{pid}"));
                    let cwd = fs::read_link(base.join("cwd")).ok();
                    cwd.as_ref()?;
                    let name = fs::read_to_string(base.join("comm")).unwrap_or_default().trim().to_string();
                    let mut open = Vec::new();
                    // The running binary and its mapped libraries are "in use" too:
                    // a venv python symlinks into ~/.pyenv, which no fd reveals.
                    if let Ok(exe) = fs::read_link(base.join("exe")) {
                        open.push(exe);
                    }
                    if with_fds {
                        if let Ok(maps) = fs::read_to_string(base.join("maps")) {
                            let home = super::home();
                            let mut seen = std::collections::HashSet::new();
                            for l in maps.lines() {
                                if let Some(i) = l.find('/') {
                                    let p = Path::new(l[i..].trim_end_matches(" (deleted)"));
                                    if p.starts_with(&home) && seen.insert(p.to_path_buf()) {
                                        open.push(p.to_path_buf());
                                    }
                                }
                            }
                        }
                        if let Ok(fds) = fs::read_dir(base.join("fd")) {
                            for fd in fds.flatten() {
                                if let Ok(t) = fs::read_link(fd.path()) {
                                    if t.is_absolute() && !t.starts_with("/dev") && !t.starts_with("/proc") {
                                        open.push(t);
                                    }
                                }
                            }
                        }
                    }
                    Some(ProcPaths { pid, name, cwd, open })
                })
                .collect()
        }

        fn threads_of(&self, pid: u32) -> Option<u32> {
            let s = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
            s.lines().find_map(|l| l.strip_prefix("Threads:")).and_then(|v| v.trim().parse().ok())
        }

        fn temp_dirs(&self) -> Vec<PathBuf> {
            let mut v = vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")];
            if let Some(t) = std::env::var_os("TMPDIR").map(PathBuf::from) {
                if !v.contains(&t) {
                    v.push(t);
                }
            }
            v
        }
    }

    /// Block devices and partitions — includes partitions that are *not* mounted.
    fn lsblk() -> Option<Vec<DiskInfo>> {
        let out = std::process::Command::new("lsblk")
            .args(["-J", "-b", "-o", "NAME,PATH,SIZE,ROTA,RM,TYPE,FSTYPE,LABEL,UUID,MOUNTPOINTS"])
            .output()
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let fstab = fs::read_to_string("/etc/fstab").unwrap_or_default();
        let fstab_has = |needle: &str, mount: Option<&PathBuf>| {
            fstab.lines().map(str::trim).filter(|l| !l.starts_with('#') && !l.is_empty()).any(|l| {
                let mut c = l.split_whitespace();
                let (src, mp) = (c.next().unwrap_or(""), c.next().unwrap_or(""));
                (!needle.is_empty() && src.contains(needle)) || mount.is_some_and(|m| Path::new(mp) == m)
            })
        };
        let mut disks = Vec::new();
        for dev in v["blockdevices"].as_array()? {
            if dev["type"] != "disk" {
                continue;
            }
            let rota = as_bool(&dev["rota"]);
            let rm = as_bool(&dev["rm"]);
            let parts: Vec<&serde_json::Value> = match dev["children"].as_array() {
                Some(c) => c.iter().collect(),
                None => vec![dev],
            };
            for p in parts {
                let fs = p["fstype"].as_str().map(str::to_string);
                if fs.as_deref() == Some("swap") || fs.is_none() {
                    continue;
                }
                let mount = p["mountpoints"]
                    .as_array()
                    .and_then(|m| m.iter().filter_map(|x| x.as_str()).min_by_key(|s| s.len()).map(PathBuf::from));
                let size = as_u64(&p["size"]);
                let (total, used, available) = match &mount {
                    Some(m) => statvfs(m).unwrap_or((size, 0, 0)),
                    None => (size, 0, 0),
                };
                let uuid = p["uuid"].as_str().map(str::to_string);
                let in_fstab = fstab_has(uuid.as_deref().unwrap_or(""), mount.as_ref().filter(|m| m.as_os_str() != "/media"));
                disks.push(DiskInfo {
                    uuid,
                    in_fstab,
                    name: p["name"].as_str().unwrap_or("?").to_string(),
                    device: p["path"].as_str().unwrap_or("?").to_string(),
                    label: p["label"].as_str().map(str::to_string),
                    fs,
                    mount,
                    total,
                    used,
                    available,
                    rotational: rota,
                    removable: rm,
                });
            }
        }
        Some(disks)
    }

    fn as_bool(v: &serde_json::Value) -> bool {
        v.as_bool().unwrap_or_else(|| v.as_str() == Some("1") || v.as_u64() == Some(1))
    }
    fn as_u64(v: &serde_json::Value) -> u64 {
        v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
    }
}
