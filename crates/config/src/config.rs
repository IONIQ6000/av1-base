//! Core configuration structures and loading logic

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::Path;

/// Error type for configuration operations
#[derive(Debug)]
pub enum ConfigError {
    /// IO error reading config file
    Io(std::io::Error),
    /// TOML parsing error
    Parse(toml::de::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "Failed to read config file: {}", e),
            ConfigError::Parse(e) => write!(f, "Failed to parse config: {}", e),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::Parse(e)
    }
}

/// CPU-related configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpuConfig {
    /// Number of logical cores (auto-detected if None)
    pub logical_cores: Option<u32>,
    /// Target CPU utilization (0.5-1.0, default 0.85)
    #[serde(default = "default_target_cpu_utilization")]
    pub target_cpu_utilization: f32,
}

fn default_target_cpu_utilization() -> f32 {
    0.85
}

impl Default for CpuConfig {
    fn default() -> Self {
        Self {
            logical_cores: None,
            target_cpu_utilization: default_target_cpu_utilization(),
        }
    }
}


/// Av1an-related configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Av1anConfig {
    /// Workers per job (0 = auto-derive)
    #[serde(default)]
    pub workers_per_job: u32,
    /// Maximum concurrent jobs (0 = auto-derive)
    #[serde(default)]
    pub max_concurrent_jobs: u32,
}

impl Default for Av1anConfig {
    fn default() -> Self {
        Self {
            workers_per_job: 0,
            max_concurrent_jobs: 0,
        }
    }
}

/// Encoder safety configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncoderSafetyConfig {
    /// Disallow hardware encoding (default true)
    #[serde(default = "default_disallow_hardware_encoding")]
    pub disallow_hardware_encoding: bool,
}

fn default_disallow_hardware_encoding() -> bool {
    true
}

impl Default for EncoderSafetyConfig {
    fn default() -> Self {
        Self {
            disallow_hardware_encoding: default_disallow_hardware_encoding(),
        }
    }
}

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub cpu: CpuConfig,
    #[serde(default)]
    pub av1an: Av1anConfig,
    #[serde(default)]
    pub encoder_safety: EncoderSafetyConfig,
}


impl Config {
    /// Load configuration from a TOML file
    ///
    /// Parses the config.toml file and handles missing optional fields with defaults.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::parse_toml(&content)
    }

    /// Parse configuration from a TOML string
    pub fn parse_toml(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content)?;
        Ok(config)
    }

    /// Apply environment variable overrides to the configuration
    ///
    /// Overrides the following values if environment variables are set:
    /// - CPU_LOGICAL_CORES -> cpu.logical_cores
    /// - CPU_TARGET_UTILIZATION -> cpu.target_cpu_utilization
    /// - AV1AN_WORKERS_PER_JOB -> av1an.workers_per_job
    /// - AV1AN_MAX_CONCURRENT_JOBS -> av1an.max_concurrent_jobs
    /// - ENCODER_DISALLOW_HARDWARE_ENCODING -> encoder_safety.disallow_hardware_encoding
    pub fn apply_env_overrides(&mut self) {
        // CPU_LOGICAL_CORES
        if let Ok(val) = env::var("CPU_LOGICAL_CORES") {
            if let Ok(cores) = val.parse::<u32>() {
                self.cpu.logical_cores = Some(cores);
            }
        }

        // CPU_TARGET_UTILIZATION
        if let Ok(val) = env::var("CPU_TARGET_UTILIZATION") {
            if let Ok(util) = val.parse::<f32>() {
                self.cpu.target_cpu_utilization = util;
            }
        }

        // AV1AN_WORKERS_PER_JOB
        if let Ok(val) = env::var("AV1AN_WORKERS_PER_JOB") {
            if let Ok(workers) = val.parse::<u32>() {
                self.av1an.workers_per_job = workers;
            }
        }

        // AV1AN_MAX_CONCURRENT_JOBS
        if let Ok(val) = env::var("AV1AN_MAX_CONCURRENT_JOBS") {
            if let Ok(jobs) = val.parse::<u32>() {
                self.av1an.max_concurrent_jobs = jobs;
            }
        }

        // ENCODER_DISALLOW_HARDWARE_ENCODING
        if let Ok(val) = env::var("ENCODER_DISALLOW_HARDWARE_ENCODING") {
            // Accept "true", "1", "yes" as true; "false", "0", "no" as false
            match val.to_lowercase().as_str() {
                "true" | "1" | "yes" => self.encoder_safety.disallow_hardware_encoding = true,
                "false" | "0" | "no" => self.encoder_safety.disallow_hardware_encoding = false,
                _ => {} // Invalid value, keep existing
            }
        }
    }

    /// Load configuration from file and apply environment overrides
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let mut config = Self::load_from_file(path)?;
        config.apply_env_overrides();
        Ok(config)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::Mutex;

    // Mutex to ensure env var tests don't interfere with each other
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Helper to clear all config-related env vars
    fn clear_env_vars() {
        env::remove_var("CPU_LOGICAL_CORES");
        env::remove_var("CPU_TARGET_UTILIZATION");
        env::remove_var("AV1AN_WORKERS_PER_JOB");
        env::remove_var("AV1AN_MAX_CONCURRENT_JOBS");
        env::remove_var("ENCODER_DISALLOW_HARDWARE_ENCODING");
    }

    // **Feature: av1-super-daemon, Property 8: Configuration Parsing and Environment Override**
    // **Validates: Requirements 8.1, 8.2, 8.3, 8.4, 8.5, 8.6**
    //
    // *For any* valid TOML configuration string and set of environment variable overrides,
    // the loaded configuration SHALL:
    // - Parse all sections (cpu, av1an, encoder_safety)
    // - Apply environment variable overrides for CPU_LOGICAL_CORES, CPU_TARGET_UTILIZATION,
    //   AV1AN_WORKERS_PER_JOB, AV1AN_MAX_CONCURRENT_JOBS, ENCODER_DISALLOW_HARDWARE_ENCODING

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_config_parses_all_sections(
            logical_cores in proptest::option::of(1u32..256),
            target_util in 0.0f32..2.0,
            workers in 0u32..64,
            max_jobs in 0u32..16,
            disallow_hw in proptest::bool::ANY,
        ) {
            // Build a valid TOML config string
            let toml_str = format!(
                r#"
[cpu]
{}
target_cpu_utilization = {}

[av1an]
workers_per_job = {}
max_concurrent_jobs = {}

[encoder_safety]
disallow_hardware_encoding = {}
"#,
                logical_cores.map(|c| format!("logical_cores = {}", c)).unwrap_or_default(),
                target_util,
                workers,
                max_jobs,
                disallow_hw
            );

            let config = Config::parse_toml(&toml_str).expect("Valid TOML should parse");

            // Verify all sections parsed correctly
            prop_assert_eq!(config.cpu.logical_cores, logical_cores);
            prop_assert!((config.cpu.target_cpu_utilization - target_util).abs() < 0.0001);
            prop_assert_eq!(config.av1an.workers_per_job, workers);
            prop_assert_eq!(config.av1an.max_concurrent_jobs, max_jobs);
            prop_assert_eq!(config.encoder_safety.disallow_hardware_encoding, disallow_hw);
        }

        #[test]
        fn prop_env_overrides_cpu_logical_cores(
            initial_cores in proptest::option::of(1u32..128),
            override_cores in 1u32..256,
        ) {
            let _guard = ENV_MUTEX.lock().unwrap();
            clear_env_vars();

            let toml_str = format!(
                r#"
[cpu]
{}
"#,
                initial_cores.map(|c| format!("logical_cores = {}", c)).unwrap_or_default()
            );

            let mut config = Config::parse_toml(&toml_str).expect("Valid TOML");
            
            // Set env var and apply override
            env::set_var("CPU_LOGICAL_CORES", override_cores.to_string());
            config.apply_env_overrides();
            clear_env_vars();

            // Env var should override the config value
            prop_assert_eq!(config.cpu.logical_cores, Some(override_cores));
        }

        #[test]
        fn prop_env_overrides_cpu_target_utilization(
            initial_util in 0.5f32..1.0,
            override_util in 0.0f32..2.0,
        ) {
            let _guard = ENV_MUTEX.lock().unwrap();
            clear_env_vars();

            let toml_str = format!(
                r#"
[cpu]
target_cpu_utilization = {}
"#,
                initial_util
            );

            let mut config = Config::parse_toml(&toml_str).expect("Valid TOML");
            
            env::set_var("CPU_TARGET_UTILIZATION", override_util.to_string());
            config.apply_env_overrides();
            clear_env_vars();

            prop_assert!((config.cpu.target_cpu_utilization - override_util).abs() < 0.0001);
        }

        #[test]
        fn prop_env_overrides_workers_per_job(
            initial_workers in 0u32..32,
            override_workers in 0u32..64,
        ) {
            let _guard = ENV_MUTEX.lock().unwrap();
            clear_env_vars();

            let toml_str = format!(
                r#"
[av1an]
workers_per_job = {}
"#,
                initial_workers
            );

            let mut config = Config::parse_toml(&toml_str).expect("Valid TOML");
            
            env::set_var("AV1AN_WORKERS_PER_JOB", override_workers.to_string());
            config.apply_env_overrides();
            clear_env_vars();

            prop_assert_eq!(config.av1an.workers_per_job, override_workers);
        }

        #[test]
        fn prop_env_overrides_max_concurrent_jobs(
            initial_jobs in 0u32..8,
            override_jobs in 0u32..16,
        ) {
            let _guard = ENV_MUTEX.lock().unwrap();
            clear_env_vars();

            let toml_str = format!(
                r#"
[av1an]
max_concurrent_jobs = {}
"#,
                initial_jobs
            );

            let mut config = Config::parse_toml(&toml_str).expect("Valid TOML");
            
            env::set_var("AV1AN_MAX_CONCURRENT_JOBS", override_jobs.to_string());
            config.apply_env_overrides();
            clear_env_vars();

            prop_assert_eq!(config.av1an.max_concurrent_jobs, override_jobs);
        }

        #[test]
        fn prop_env_overrides_disallow_hardware_encoding(
            initial_disallow in proptest::bool::ANY,
            override_disallow in proptest::bool::ANY,
        ) {
            let _guard = ENV_MUTEX.lock().unwrap();
            clear_env_vars();

            let toml_str = format!(
                r#"
[encoder_safety]
disallow_hardware_encoding = {}
"#,
                initial_disallow
            );

            let mut config = Config::parse_toml(&toml_str).expect("Valid TOML");
            
            // Test with "true"/"false" string format
            env::set_var("ENCODER_DISALLOW_HARDWARE_ENCODING", override_disallow.to_string());
            config.apply_env_overrides();
            clear_env_vars();

            prop_assert_eq!(config.encoder_safety.disallow_hardware_encoding, override_disallow);
        }
    }

    // Test that missing sections use defaults
    #[test]
    fn test_empty_config_uses_defaults() {
        let config = Config::parse_toml("").expect("Empty TOML should parse");
        
        assert_eq!(config.cpu.logical_cores, None);
        assert!((config.cpu.target_cpu_utilization - 0.85).abs() < 0.0001);
        assert_eq!(config.av1an.workers_per_job, 0);
        assert_eq!(config.av1an.max_concurrent_jobs, 0);
        assert!(config.encoder_safety.disallow_hardware_encoding);
    }

    // Test partial config with some sections missing
    #[test]
    fn test_partial_config_uses_defaults_for_missing() {
        let toml_str = r#"
[cpu]
logical_cores = 16
"#;
        let config = Config::parse_toml(toml_str).expect("Partial TOML should parse");
        
        assert_eq!(config.cpu.logical_cores, Some(16));
        assert!((config.cpu.target_cpu_utilization - 0.85).abs() < 0.0001); // default
        assert_eq!(config.av1an.workers_per_job, 0); // default
        assert_eq!(config.av1an.max_concurrent_jobs, 0); // default
        assert!(config.encoder_safety.disallow_hardware_encoding); // default
    }
}
