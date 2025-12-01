//! Concurrency planning module for AV1 Super Daemon
//!
//! Derives optimal encoding concurrency settings from CPU core count and configuration.

use crate::config::Config;

/// Concurrency plan derived from configuration and system resources
#[derive(Debug, Clone, PartialEq)]
pub struct ConcurrencyPlan {
    /// Total logical CPU cores available
    pub total_cores: u32,
    /// Target number of threads to use based on utilization
    pub target_threads: u32,
    /// Number of Av1an workers per encoding job
    pub av1an_workers: u32,
    /// Maximum number of concurrent encoding jobs
    pub max_concurrent_jobs: u32,
}

impl ConcurrencyPlan {
    /// Derive a concurrency plan from configuration
    ///
    /// Uses the following rules:
    /// - Detects CPU cores via num_cpus if not specified in config
    /// - Derives av1an_workers: 8 for 32+ cores, 4 otherwise (unless explicit)
    /// - Derives max_concurrent_jobs: 1 for 24+ cores, 2 otherwise (unless explicit)
    /// - Clamps target_cpu_utilization to [0.5, 1.0]
    pub fn derive(cfg: &Config) -> Self {
        // Get core count: use config value or auto-detect
        let total_cores = cfg
            .cpu
            .logical_cores
            .unwrap_or_else(|| num_cpus::get() as u32);

        // Clamp utilization to [0.5, 1.0]
        let clamped_utilization = clamp_utilization(cfg.cpu.target_cpu_utilization);

        // Calculate target threads based on utilization
        let target_threads = ((total_cores as f32) * clamped_utilization).round() as u32;

        // Derive av1an_workers: use explicit value if non-zero, otherwise derive
        let av1an_workers = if cfg.av1an.workers_per_job > 0 {
            cfg.av1an.workers_per_job
        } else {
            derive_workers(total_cores)
        };

        // Derive max_concurrent_jobs: use explicit value if non-zero, otherwise derive
        let max_concurrent_jobs = if cfg.av1an.max_concurrent_jobs > 0 {
            cfg.av1an.max_concurrent_jobs
        } else {
            derive_max_jobs(total_cores)
        };

        Self {
            total_cores,
            target_threads,
            av1an_workers,
            max_concurrent_jobs,
        }
    }
}

/// Derive worker count based on core count
/// - 8 workers for 32+ cores
/// - 4 workers otherwise
fn derive_workers(cores: u32) -> u32 {
    if cores >= 32 {
        8
    } else {
        4
    }
}

/// Derive max concurrent jobs based on core count
/// - 1 job for 24+ cores
/// - 2 jobs otherwise
fn derive_max_jobs(cores: u32) -> u32 {
    if cores >= 24 {
        1
    } else {
        2
    }
}

/// Clamp utilization to valid range [0.5, 1.0]
fn clamp_utilization(util: f32) -> f32 {
    util.clamp(0.5, 1.0)
}

/// Public function to derive a concurrency plan from configuration
pub fn derive_plan(cfg: &Config) -> ConcurrencyPlan {
    ConcurrencyPlan::derive(cfg)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Av1anConfig, CpuConfig, EncoderSafetyConfig};
    use proptest::prelude::*;

    // **Feature: av1-super-daemon, Property 1: Concurrency Plan Derivation**
    // **Validates: Requirements 1.1, 1.2, 1.3**
    //
    // *For any* CPU configuration with `logical_cores` and no explicit worker/job settings,
    // the derived concurrency plan SHALL:
    // - Set `av1an_workers = 8` when cores >= 32, otherwise `av1an_workers = 4`
    // - Set `max_concurrent_jobs = 1` when cores >= 24, otherwise `max_concurrent_jobs = 2`
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_concurrency_derivation(
            cores in 1u32..256,
        ) {
            let cfg = Config {
                cpu: CpuConfig {
                    logical_cores: Some(cores),
                    target_cpu_utilization: 0.85,
                },
                av1an: Av1anConfig {
                    workers_per_job: 0,      // auto-derive
                    max_concurrent_jobs: 0,  // auto-derive
                },
                encoder_safety: EncoderSafetyConfig::default(),
            };

            let plan = derive_plan(&cfg);

            // Verify core count is preserved
            prop_assert_eq!(plan.total_cores, cores);

            // Verify worker derivation: 8 for 32+ cores, 4 otherwise
            let expected_workers = if cores >= 32 { 8 } else { 4 };
            prop_assert_eq!(
                plan.av1an_workers, expected_workers,
                "For {} cores, expected {} workers but got {}",
                cores, expected_workers, plan.av1an_workers
            );

            // Verify max_concurrent_jobs derivation: 1 for 24+ cores, 2 otherwise
            let expected_jobs = if cores >= 24 { 1 } else { 2 };
            prop_assert_eq!(
                plan.max_concurrent_jobs, expected_jobs,
                "For {} cores, expected {} max jobs but got {}",
                cores, expected_jobs, plan.max_concurrent_jobs
            );
        }
    }

    // **Feature: av1-super-daemon, Property 2: Explicit Configuration Override**
    // **Validates: Requirements 1.4**
    //
    // *For any* configuration with explicit non-zero `workers_per_job` or `max_concurrent_jobs`
    // values, the derived concurrency plan SHALL use those explicit values unchanged.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_explicit_config_override(
            cores in 1u32..256,
            explicit_workers in 1u32..64,
            explicit_jobs in 1u32..16,
        ) {
            let cfg = Config {
                cpu: CpuConfig {
                    logical_cores: Some(cores),
                    target_cpu_utilization: 0.85,
                },
                av1an: Av1anConfig {
                    workers_per_job: explicit_workers,
                    max_concurrent_jobs: explicit_jobs,
                },
                encoder_safety: EncoderSafetyConfig::default(),
            };

            let plan = derive_plan(&cfg);

            // Explicit values should be used unchanged
            prop_assert_eq!(
                plan.av1an_workers, explicit_workers,
                "Explicit workers {} should be preserved, got {}",
                explicit_workers, plan.av1an_workers
            );
            prop_assert_eq!(
                plan.max_concurrent_jobs, explicit_jobs,
                "Explicit max_jobs {} should be preserved, got {}",
                explicit_jobs, plan.max_concurrent_jobs
            );
        }
    }

    // **Feature: av1-super-daemon, Property 3: Utilization Clamping**
    // **Validates: Requirements 1.5**
    //
    // *For any* `target_cpu_utilization` value, the effective utilization used in
    // concurrency planning SHALL be clamped to the range [0.5, 1.0].
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_utilization_clamping(
            cores in 1u32..256,
            raw_utilization in -1.0f32..3.0,
        ) {
            let cfg = Config {
                cpu: CpuConfig {
                    logical_cores: Some(cores),
                    target_cpu_utilization: raw_utilization,
                },
                av1an: Av1anConfig::default(),
                encoder_safety: EncoderSafetyConfig::default(),
            };

            let plan = derive_plan(&cfg);

            // Calculate expected clamped utilization
            let clamped = raw_utilization.clamp(0.5, 1.0);
            let expected_target_threads = ((cores as f32) * clamped).round() as u32;

            // Verify target_threads reflects clamped utilization
            prop_assert_eq!(
                plan.target_threads, expected_target_threads,
                "For {} cores and {} utilization (clamped to {}), expected {} target threads but got {}",
                cores, raw_utilization, clamped, expected_target_threads, plan.target_threads
            );

            // Verify target_threads is within valid bounds
            let min_threads = ((cores as f32) * 0.5).round() as u32;
            let max_threads = cores;
            prop_assert!(
                plan.target_threads >= min_threads && plan.target_threads <= max_threads,
                "target_threads {} should be in range [{}, {}]",
                plan.target_threads, min_threads, max_threads
            );
        }
    }
}
