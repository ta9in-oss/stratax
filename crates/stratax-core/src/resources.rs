//! Live CPU / memory / process sampling. Keep one `Sampler` alive and call
//! `sample()` on an interval — CPU usage needs two readings to be meaningful.

use crate::agents::{classify, AgentActivity, AgentKind};
use crate::platform::{self, Platform};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, RefreshKind, System, UpdateKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub cmd: String,
    pub cwd: Option<PathBuf>,
    pub cpu: f32,
    pub memory: u64,
    pub threads: u32,
    pub status: String,
    pub agent: Option<String>,
    pub run_secs: u64,
    pub disk_read: u64,
    pub disk_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub at: i64,
    pub cpu_total: f32,
    pub cpu_cores: Vec<f32>,
    pub cpu_brand: String,
    pub load: [f64; 3],
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub uptime: u64,
    pub process_count: usize,
    pub thread_count: u64,
    pub zombies: usize,
    pub processes: Vec<ProcInfo>,
    pub agents: Vec<AgentActivity>,
}

pub struct Sampler {
    sys: System,
    plat: Box<dyn Platform>,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(RefreshKind::everything());
        Self { sys, plat: platform::current() }
    }

    pub fn sample(&mut self) -> ResourceSnapshot {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet),
        );
        let cores: Vec<f32> = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let la = System::load_average();
        let mut processes = Vec::with_capacity(self.sys.processes().len());
        let mut zombies = 0;
        let mut threads_total = 0u64;
        for (pid, p) in self.sys.processes() {
            // sysinfo lists threads as processes on Linux; skip them.
            if p.thread_kind().is_some() {
                continue;
            }
            if p.status() == ProcessStatus::Zombie {
                zombies += 1;
            }
            let pid = pid.as_u32();
            let cmd: Vec<String> = p.cmd().iter().map(|s| s.to_string_lossy().into_owned()).collect();
            let name = p.name().to_string_lossy().into_owned();
            let threads = self.plat.threads_of(pid).unwrap_or(1);
            threads_total += threads as u64;
            let du = p.disk_usage();
            processes.push(ProcInfo {
                pid,
                ppid: p.parent().map(|x| x.as_u32()),
                agent: classify(&name, &cmd).map(|(a, _)| a.to_string()),
                cmd: cmd.join(" "),
                name,
                cwd: p.cwd().map(|c| c.to_path_buf()),
                cpu: p.cpu_usage(),
                memory: p.memory(),
                threads,
                status: format!("{:?}", p.status()),
                run_secs: p.run_time(),
                disk_read: du.read_bytes,
                disk_write: du.written_bytes,
            });
        }
        processes.sort_unstable_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.memory.cmp(&a.memory)));
        let agents = group_agents(&processes);
        ResourceSnapshot {
            at: now(),
            cpu_total: self.sys.global_cpu_usage(),
            cpu_brand: self.sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
            cpu_cores: cores,
            load: [la.one, la.five, la.fifteen],
            mem_total: self.sys.total_memory(),
            mem_used: self.sys.used_memory(),
            mem_available: self.sys.available_memory(),
            swap_total: self.sys.total_swap(),
            swap_used: self.sys.used_swap(),
            uptime: System::uptime(),
            process_count: processes.len(),
            thread_count: threads_total,
            zombies,
            processes,
            agents,
        }
    }
}

pub fn group_agents(procs: &[ProcInfo]) -> Vec<AgentActivity> {
    let mut map: HashMap<String, AgentActivity> = HashMap::new();
    for p in procs {
        let Some(a) = &p.agent else { continue };
        let kind = classify(&p.name, &[p.cmd.clone()]).map(|x| x.1).unwrap_or(AgentKind::Workload);
        let e = map.entry(a.clone()).or_insert_with(|| AgentActivity {
            agent: a.clone(),
            kind,
            processes: 0,
            threads: 0,
            cpu: 0.0,
            memory: 0,
            cwds: vec![],
            footprint: 0,
        });
        e.processes += 1;
        e.threads += p.threads as u64;
        e.cpu += p.cpu;
        e.memory += p.memory;
        if let Some(c) = &p.cwd {
            if c != &PathBuf::from("/") && !c.starts_with("/proc") && !e.cwds.contains(c) && e.cwds.len() < 24 {
                e.cwds.push(c.clone());
            }
        }
    }
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by(|a, b| b.memory.cmp(&a.memory));
    v
}

/// Send SIGTERM (or SIGKILL when `force`).
pub fn signal(pid: u32, force: bool) -> anyhow::Result<()> {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    if unsafe { libc::kill(pid as i32, sig) } != 0 {
        anyhow::bail!("kill {pid}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
