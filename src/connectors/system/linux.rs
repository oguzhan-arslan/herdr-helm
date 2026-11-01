//! Linux: /proc/stat sampled over a second for the CPU, /proc/meminfo for the memory.

use super::{apps_of, App};
use crate::util::exec;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

pub struct Linux;

/// (busy, total) jiffies of all CPUs.
fn cpu_times() -> Result<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat")?;
    let line = stat.lines().next().ok_or_else(|| anyhow!("empty /proc/stat"))?;
    let nums: Vec<u64> = line.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect();
    let idle = nums.get(3).copied().unwrap_or(0) + nums.get(4).copied().unwrap_or(0);
    let total: u64 = nums.iter().sum();
    Ok((total - idle, total))
}

/// (total, available) bytes of memory.
fn memory() -> Result<(u64, u64)> {
    let info = std::fs::read_to_string("/proc/meminfo")?;
    let kb = |key: &str| {
        info.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0)
            * 1024
    };
    Ok((kb("MemTotal:"), kb("MemAvailable:")))
}

impl super::System for Linux {
    fn name(&self) -> String {
        "linux".into()
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["usage".into(), "apps".into()]
    }
    fn usage(&self) -> Result<Value> {
        let (b1, t1) = cpu_times()?;
        std::thread::sleep(std::time::Duration::from_secs(1));
        let (b2, t2) = cpu_times()?;
        let cpu = if t2 > t1 { (b2 - b1) as f64 * 100.0 / (t2 - t1) as f64 } else { 0.0 };
        let (total, available) = memory()?;
        let mem = if total > 0 { (total - available) as f64 * 100.0 / total as f64 } else { 0.0 };
        Ok(json!({ "cpu": cpu, "mem": mem, "total": total }))
    }
    fn apps(&self) -> Result<(Vec<App>, u64, u64)> {
        let ps = exec("ps", &["-Aeo", "pcpu=,rss=,comm="], None, None)?;
        let cores = std::thread::available_parallelism().map(|n| n.get() as u64).unwrap_or(1);
        Ok((apps_of(&ps.stdout), cores, memory()?.0))
    }
}
