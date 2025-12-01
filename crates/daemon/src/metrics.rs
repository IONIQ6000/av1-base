//! Metrics module for AV1 Super Daemon
//!
//! Provides structs for job metrics, system metrics, and metrics snapshots
//! with JSON serialization support.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Per-job metrics tracking encoding progress and statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobMetrics {
    pub id: String,
    pub input_path: String,
    pub stage: String,
    pub progress: f32,
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub crf: u8,
    pub encoder: String,
    pub workers: u32,
    pub est_remaining_secs: f32,
    pub frames_encoded: u64,
    pub total_frames: u64,
    pub size_in_bytes_before: u64,
    pub size_in_bytes_after: u64,
    pub vmaf: Option<f32>,
    pub psnr: Option<f32>,
    pub ssim: Option<f32>,
}

/// System-level metrics for resource monitoring
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemMetrics {
    pub cpu_usage_percent: f32,
    pub mem_usage_percent: f32,
    pub load_avg_1: f32,
    pub load_avg_5: f32,
    pub load_avg_15: f32,
}

/// Complete metrics snapshot including jobs, system, and aggregate stats
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsSnapshot {
    pub timestamp_unix_ms: i64,
    pub jobs: Vec<JobMetrics>,
    pub system: SystemMetrics,
    pub queue_len: usize,
    pub running_jobs: usize,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub total_bytes_encoded: u64,
}


/// Shared metrics state for concurrent access across daemon components
pub type SharedMetrics = Arc<RwLock<MetricsSnapshot>>;

impl Default for SystemMetrics {
    fn default() -> Self {
        Self {
            cpu_usage_percent: 0.0,
            mem_usage_percent: 0.0,
            load_avg_1: 0.0,
            load_avg_5: 0.0,
            load_avg_15: 0.0,
        }
    }
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            timestamp_unix_ms: 0,
            jobs: Vec::new(),
            system: SystemMetrics::default(),
            queue_len: 0,
            running_jobs: 0,
            completed_jobs: 0,
            failed_jobs: 0,
            total_bytes_encoded: 0,
        }
    }
}

/// Creates a new SharedMetrics instance with default values
pub fn new_shared_metrics() -> SharedMetrics {
    Arc::new(RwLock::new(MetricsSnapshot::default()))
}

/// Collects current system metrics using sysinfo
pub fn collect_system_metrics() -> SystemMetrics {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cpu_usage = sys.global_cpu_usage();
    let total_memory = sys.total_memory();
    let used_memory = sys.used_memory();
    let mem_usage = if total_memory > 0 {
        (used_memory as f64 / total_memory as f64 * 100.0) as f32
    } else {
        0.0
    };

    let load_avg = System::load_average();

    SystemMetrics {
        cpu_usage_percent: cpu_usage,
        mem_usage_percent: mem_usage,
        load_avg_1: load_avg.one as f32,
        load_avg_5: load_avg.five as f32,
        load_avg_15: load_avg.fifteen as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // **Feature: av1-super-daemon, Property 7: MetricsSnapshot Serialization Round-Trip**
    // **Validates: Requirements 7.2, 7.3, 7.4, 7.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_metrics_snapshot_round_trip(
            timestamp in any::<i64>(),
            queue_len in 0usize..1000,
            running_jobs in 0usize..100,
            completed_jobs in any::<u64>(),
            failed_jobs in any::<u64>(),
            total_bytes_encoded in any::<u64>(),
            cpu_usage in 0.0f32..100.0,
            mem_usage in 0.0f32..100.0,
            load_1 in 0.0f32..100.0,
            load_5 in 0.0f32..100.0,
            load_15 in 0.0f32..100.0,
            job_count in 0usize..5,
        ) {
            let jobs: Vec<JobMetrics> = (0..job_count).map(|i| JobMetrics {
                id: format!("job-{}", i),
                input_path: format!("/path/to/video{}.mkv", i),
                stage: "encoding".to_string(),
                progress: 0.5,
                fps: 12.5,
                bitrate_kbps: 8500.0,
                crf: 8,
                encoder: "svt-av1".to_string(),
                workers: 8,
                est_remaining_secs: 3600.0,
                frames_encoded: 54000,
                total_frames: 120000,
                size_in_bytes_before: 5368709120,
                size_in_bytes_after: 2147483648,
                vmaf: Some(95.5),
                psnr: Some(45.2),
                ssim: Some(0.98),
            }).collect();

            let snapshot = MetricsSnapshot {
                timestamp_unix_ms: timestamp,
                jobs,
                system: SystemMetrics {
                    cpu_usage_percent: cpu_usage,
                    mem_usage_percent: mem_usage,
                    load_avg_1: load_1,
                    load_avg_5: load_5,
                    load_avg_15: load_15,
                },
                queue_len,
                running_jobs,
                completed_jobs,
                failed_jobs,
                total_bytes_encoded,
            };

            // Serialize to JSON
            let json = serde_json::to_string(&snapshot).expect("serialization should succeed");

            // Deserialize back
            let deserialized: MetricsSnapshot = serde_json::from_str(&json)
                .expect("deserialization should succeed");

            // Verify round-trip produces equivalent snapshot
            prop_assert_eq!(snapshot, deserialized);
        }
    }
}
