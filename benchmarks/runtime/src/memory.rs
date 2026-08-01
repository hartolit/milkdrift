//! Safe Linux process-memory and machine-information parsing.

use std::collections::BTreeSet;
#[cfg(target_os = "linux")]
use std::fs;

use serde::Serialize;

use crate::error::{BenchmarkError, BenchmarkResult};

const KIBIBYTE_BYTES: u64 = 1_024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ProcessMemory {
    pub(crate) vm_rss_bytes: Option<u64>,
    pub(crate) vm_hwm_bytes: Option<u64>,
}

pub(crate) fn process_memory() -> BenchmarkResult<ProcessMemory> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").map_err(|error| {
            BenchmarkError::new(format!("could not read /proc/self/status: {error}"))
        })?;
        parse_proc_status(&status)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(ProcessMemory::default())
    }
}

pub(crate) fn total_memory_bytes() -> BenchmarkResult<Option<u64>> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = fs::read_to_string("/proc/meminfo").map_err(|error| {
            BenchmarkError::new(format!("could not read /proc/meminfo: {error}"))
        })?;
        parse_kib_field(&meminfo, "MemTotal")
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}

pub(crate) fn cpu_information() -> BenchmarkResult<(Option<String>, Option<usize>)> {
    #[cfg(target_os = "linux")]
    {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").map_err(|error| {
            BenchmarkError::new(format!("could not read /proc/cpuinfo: {error}"))
        })?;
        Ok((
            parse_cpu_model(&cpuinfo),
            parse_physical_cpu_count(&cpuinfo),
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok((None, None))
    }
}

fn parse_proc_status(input: &str) -> BenchmarkResult<ProcessMemory> {
    Ok(ProcessMemory {
        vm_rss_bytes: parse_kib_field(input, "VmRSS")?,
        vm_hwm_bytes: parse_kib_field(input, "VmHWM")?,
    })
}

fn parse_kib_field(input: &str, field: &str) -> BenchmarkResult<Option<u64>> {
    let prefix = format!("{field}:");
    let mut parsed = None;
    for line in input.lines() {
        let Some(rest) = line.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if parsed.is_some() {
            return Err(BenchmarkError::new(format!(
                "{field} appears more than once in a proc file"
            )));
        }
        let mut parts = rest.split_whitespace();
        let value = parts
            .next()
            .ok_or_else(|| BenchmarkError::new(format!("{field} has no numeric value")))?
            .parse::<u64>()
            .map_err(|error| BenchmarkError::new(format!("{field} is not numeric: {error}")))?;
        let unit = parts
            .next()
            .ok_or_else(|| BenchmarkError::new(format!("{field} has no unit")))?;
        if unit != "kB" || parts.next().is_some() {
            return Err(BenchmarkError::new(format!(
                "{field} must contain exactly one kB value"
            )));
        }
        parsed =
            Some(value.checked_mul(KIBIBYTE_BYTES).ok_or_else(|| {
                BenchmarkError::new(format!("{field} byte conversion overflowed"))
            })?);
    }
    Ok(parsed)
}

fn parse_cpu_model(input: &str) -> Option<String> {
    input.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        if key == "model name" || key == "Hardware" || key == "Processor" {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_owned())
            }
        } else {
            None
        }
    })
}

fn parse_physical_cpu_count(input: &str) -> Option<usize> {
    let mut pairs = BTreeSet::new();
    for block in input.split("\n\n") {
        let mut physical_id = None;
        let mut core_id = None;
        for line in block.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            match key.trim() {
                "physical id" => physical_id = value.trim().parse::<u32>().ok(),
                "core id" => core_id = value.trim().parse::<u32>().ok(),
                _ => {}
            }
        }
        if let (Some(physical), Some(core)) = (physical_id, core_id) {
            pairs.insert((physical, core));
        }
    }
    (!pairs.is_empty()).then_some(pairs.len())
}

#[cfg(test)]
mod tests {
    use super::{ProcessMemory, parse_cpu_model, parse_physical_cpu_count, parse_proc_status};

    #[test]
    fn proc_status_parser_converts_kibibytes_without_unsafe_code() -> Result<(), String> {
        let input = "Name:\tbaseline\nVmHWM:\t  2048 kB\nVmRSS:\t1024 kB\nThreads:\t2\n";
        let parsed = parse_proc_status(input).map_err(|error| error.to_string())?;
        assert_eq!(
            parsed,
            ProcessMemory {
                vm_rss_bytes: Some(1_048_576),
                vm_hwm_bytes: Some(2_097_152),
            }
        );
        Ok(())
    }

    #[test]
    fn proc_status_parser_allows_unavailable_fields() -> Result<(), String> {
        let parsed = parse_proc_status("Name:\tbaseline\n").map_err(|error| error.to_string())?;
        assert_eq!(parsed, ProcessMemory::default());
        Ok(())
    }

    #[test]
    fn proc_status_parser_rejects_wrong_units_and_duplicates() {
        assert!(parse_proc_status("VmRSS: 4 MB\n").is_err());
        assert!(parse_proc_status("VmRSS: 4 kB\nVmRSS: 5 kB\n").is_err());
    }

    #[test]
    fn cpuinfo_parsers_extract_model_and_unique_physical_cores() {
        let input = "processor: 0\nmodel name: Example CPU\nphysical id: 0\ncore id: 0\n\nprocessor: 1\nmodel name: Example CPU\nphysical id: 0\ncore id: 0\n\nprocessor: 2\nmodel name: Example CPU\nphysical id: 0\ncore id: 1\n";
        assert_eq!(parse_cpu_model(input).as_deref(), Some("Example CPU"));
        assert_eq!(parse_physical_cpu_count(input), Some(2));
    }
}
