//! macOS: iostat for the CPU, memory_pressure for the memory, sysctl for the size.

use super::{apps_of, App};
use crate::util::exec;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

pub struct MacOs;

const USAGE: &str = "iostat -n0 -c 2 -w 1 | tail -1 | awk '{ print 100 - $3 }'
memory_pressure -Q | awk -F': ' '/free percentage/ { gsub(\"%\",\"\",$2); print 100-$2 }'
sysctl -n hw.memsize";

impl super::System for MacOs {
    fn name(&self) -> String {
        "macos".into()
    }
    fn capabilities(&self) -> Vec<String> {
        vec!["usage".into(), "apps".into()]
    }
    fn usage(&self) -> Result<Value> {
        let out = exec("sh", &["-c", USAGE], None, None)?;
        let nums: Vec<f64> = out.stdout.lines().filter_map(|l| l.trim().parse().ok()).collect();
        match nums.as_slice() {
            [cpu, mem, total] if out.ok() => Ok(json!({ "cpu": cpu, "mem": mem, "total": *total as u64 })),
            _ => Err(anyhow!("could not read the CPU and memory: {}", out.stderr.trim())),
        }
    }
    fn apps(&self) -> Result<(Vec<App>, u64, u64)> {
        let ps = exec("ps", &["-Aeo", "pcpu=,rss=,comm="], None, None)?;
        let info = exec("sysctl", &["-n", "hw.ncpu", "hw.memsize"], None, None)?;
        let mut nums = info.stdout.lines().filter_map(|l| l.trim().parse::<u64>().ok());
        Ok((apps_of(&ps.stdout), nums.next().unwrap_or(1), nums.next().unwrap_or(1)))
    }
}
