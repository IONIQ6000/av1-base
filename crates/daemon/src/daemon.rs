//! Daemon startup and main loop for AV1 Super Daemon
//!
//! Provides the daemon entry point, startup sequence, and main processing loop.

use crate::classify::classify_source;
use crate::config::{Config, ConfigError};
use crate::concurrency::{derive_plan, ConcurrencyPlan};
use crate::gates::{check_gates, probe_file, GateResult, GatesConfig as DaemonGatesConfig};
use crate::job_executor::{Job, JobError, JobExecutor};
use crate::jobs::{create_job, job_exists_for_path, load_jobs, save_job};
use crate::metrics::{collect_system_metrics, new_shared_metrics, SharedMetrics};
use crate::metrics_server::run_metrics_server;
use crate::scan::scan_libraries;
use crate::skip_marker::{write_skip_marker, write_why_sidecar};
use crate::stability::{check_stability, StabilityResult};
use crate::startup::{run_startup_checks, StartupError};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

/// Error type for daemon operations
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// Startup check failed
    #[error("Startup check failed: {0}")]
    Startup(#[from] StartupError),

    /// Job execution error
    #[error("Job execution error: {0}")]
    Job(#[from] JobError),

    /// Server error
    #[error("Server error: {0}")]
    Server(String),

    /// IO error (e.g., directory creation)
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

/// Creates required directories for daemon operation.
///
/// Creates the job_state_dir and temp_output_dir if they don't exist.
///
/// # Arguments
/// * `config` - The daemon configuration containing path settings
///
/// # Returns
/// * `Ok(())` - Directories created or already exist
/// * `Err(io::Error)` - Failed to create directories
///
/// # Requirements
/// - 14.1: Job state directory must exist for persisting job JSON files
pub fn create_required_directories(config: &Config) -> Result<(), io::Error> {
    // Create job_state_dir if not exists
    fs::create_dir_all(&config.paths.job_state_dir)?;

    // Create temp_output_dir if not exists
    fs::create_dir_all(&config.paths.temp_output_dir)?;

    Ok(())
}

/// Daemon state containing all runtime components
pub struct Daemon {
    /// Configuration loaded from file and environment
    pub config: Config,
    /// Derived concurrency plan
    pub concurrency_plan: ConcurrencyPlan,
    /// Shared metrics state
    pub metrics: SharedMetrics,
    /// Job executor for processing encoding jobs
    pub executor: Arc<JobExecutor>,
    /// Job queue sender
    job_tx: mpsc::Sender<Job>,
    /// Job queue receiver (wrapped for async access)
    job_rx: Arc<RwLock<mpsc::Receiver<Job>>>,
}

impl Daemon {
    /// Initialize the daemon with configuration from file
    ///
    /// This performs the full startup sequence:
    /// 1. Load config from file
    /// 2. Apply environment overrides
    /// 3. Run startup checks (software-only, av1an, ffmpeg)
    /// 4. Create required directories (job_state_dir, temp_output_dir)
    /// 5. Derive concurrency plan
    /// 6. Initialize shared metrics
    ///
    /// # Arguments
    /// * `config_path` - Path to the config.toml file
    /// * `temp_base_dir` - Base directory for temporary chunk files
    ///
    /// # Returns
    /// * `Ok(Daemon)` - Daemon initialized successfully
    /// * `Err(DaemonError)` - Initialization failed
    ///
    /// # Requirements
    /// - 4.1: Verify av1an --version executes successfully
    /// - 4.3: Verify FFmpeg version is 8.0 or newer
    /// - 3.1: Reject configuration with hardware encoder flags when disallow_hardware_encoding is enabled
    /// - 14.1: Create job_state_dir for persisting job JSON files
    pub async fn new<P: AsRef<Path>>(
        config_path: P,
        temp_base_dir: PathBuf,
    ) -> Result<Self, DaemonError> {
        // Step 1 & 2: Load config from file and apply environment overrides
        let config = Config::load(config_path)?;

        // Step 3: Run startup checks in order: software-only, av1an, ffmpeg
        run_startup_checks(&config)?;

        // Step 4: Create required directories
        create_required_directories(&config)?;

        // Step 5: Derive concurrency plan from configuration
        let concurrency_plan = derive_plan(&config);

        // Step 6: Initialize shared metrics
        let metrics = new_shared_metrics();

        // Create job executor
        let executor = Arc::new(JobExecutor::new(
            concurrency_plan.clone(),
            metrics.clone(),
            temp_base_dir,
        ));

        // Create job queue channel
        let (job_tx, job_rx) = mpsc::channel(100);

        Ok(Self {
            config,
            concurrency_plan,
            metrics,
            executor,
            job_tx,
            job_rx: Arc::new(RwLock::new(job_rx)),
        })
    }

    /// Initialize the daemon with an existing configuration
    ///
    /// Useful for testing or when configuration is already loaded.
    pub async fn with_config(config: Config, temp_base_dir: PathBuf) -> Result<Self, DaemonError> {
        // Run startup checks
        run_startup_checks(&config)?;

        // Create required directories
        create_required_directories(&config)?;

        // Derive concurrency plan
        let concurrency_plan = derive_plan(&config);

        // Initialize shared metrics
        let metrics = new_shared_metrics();

        // Create job executor
        let executor = Arc::new(JobExecutor::new(
            concurrency_plan.clone(),
            metrics.clone(),
            temp_base_dir,
        ));

        // Create job queue channel
        let (job_tx, job_rx) = mpsc::channel(100);

        Ok(Self {
            config,
            concurrency_plan,
            metrics,
            executor,
            job_tx,
            job_rx: Arc::new(RwLock::new(job_rx)),
        })
    }

    /// Initialize the daemon without running startup checks
    ///
    /// Useful for testing when external tools (av1an, ffmpeg) are not available.
    pub fn new_without_checks(config: Config, temp_base_dir: PathBuf) -> Self {
        let concurrency_plan = derive_plan(&config);
        let metrics = new_shared_metrics();
        let executor = Arc::new(JobExecutor::new(
            concurrency_plan.clone(),
            metrics.clone(),
            temp_base_dir,
        ));
        let (job_tx, job_rx) = mpsc::channel(100);

        Self {
            config,
            concurrency_plan,
            metrics,
            executor,
            job_tx,
            job_rx: Arc::new(RwLock::new(job_rx)),
        }
    }

    /// Submit a job to the queue
    pub async fn submit_job(&self, job: Job) -> Result<(), DaemonError> {
        self.job_tx
            .send(job)
            .await
            .map_err(|e| DaemonError::Server(format!("Failed to submit job: {}", e)))
    }

    /// Get a clone of the job sender for external job submission
    pub fn job_sender(&self) -> mpsc::Sender<Job> {
        self.job_tx.clone()
    }

    /// Get the shared metrics
    pub fn metrics(&self) -> SharedMetrics {
        self.metrics.clone()
    }

    /// Start the metrics HTTP server
    ///
    /// Spawns the HTTP server as a background task.
    ///
    /// # Requirements
    /// - 7.1: Start HTTP server on 127.0.0.1:7878
    pub fn start_metrics_server(&self) -> tokio::task::JoinHandle<()> {
        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(metrics).await {
                eprintln!("Metrics server error: {}", e);
            }
        })
    }

    /// Start the metrics update task
    ///
    /// Periodically updates system metrics in the shared state.
    pub fn start_metrics_updater(&self) -> tokio::task::JoinHandle<()> {
        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            loop {
                // Collect and update system metrics
                let system_metrics = collect_system_metrics();
                {
                    let mut snapshot = metrics.write().await;
                    snapshot.system = system_metrics;
                    snapshot.timestamp_unix_ms = chrono_timestamp_ms();
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
    }

    /// Run the daemon main loop
    ///
    /// Processes jobs from the queue and updates metrics on completion.
    ///
    /// # Requirements
    /// - 5.2: Proceed to validation after successful encoding
    /// - 5.3: Mark job as failed and halt processing on encoding failure
    /// - 5.4: Replace original file after validation passes
    pub async fn run(&self) -> Result<(), DaemonError> {
        loop {
            // Get next job from queue
            let job = {
                let mut rx = self.job_rx.write().await;
                rx.recv().await
            };

            match job {
                Some(job) => {
                    // Update queue length in metrics
                    {
                        let mut metrics = self.metrics.write().await;
                        metrics.queue_len = metrics.queue_len.saturating_sub(1);
                    }

                    // Execute the job
                    let executor = self.executor.clone();
                    let metrics = self.metrics.clone();

                    // Spawn job execution as a separate task
                    tokio::spawn(async move {
                        match executor.execute(job).await {
                            Ok(completed_job) => {
                                // Update total bytes encoded on success
                                if let Ok(metadata) =
                                    std::fs::metadata(&completed_job.output_path)
                                {
                                    let mut m = metrics.write().await;
                                    m.total_bytes_encoded += metadata.len();
                                }
                            }
                            Err(e) => {
                                eprintln!("Job execution failed: {}", e);
                            }
                        }
                    });
                }
                None => {
                    // Channel closed, exit loop
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run a single scan cycle to discover and queue new encoding jobs.
    ///
    /// This method implements the scan cycle:
    /// 1. Load existing jobs to avoid duplicates
    /// 2. Scan all library_roots for video files
    /// 3. For each candidate: stability check, probe, gates, classify, create job
    /// 4. Queue jobs for execution
    ///
    /// # Requirements
    /// - 11.1: Recursively walk each configured library_root directory
    /// - 12.1-12.4: Verify file stability before processing
    /// - 13.1-13.6: Gate files based on probe results
    /// - 14.3: Load existing jobs to avoid duplicate work
    /// - 15.1-15.5: Classify source files
    pub async fn run_scan_cycle(&self) -> Result<usize, DaemonError> {
        let mut jobs_queued = 0;

        // Step 1: Load existing jobs to avoid duplicates (Requirement 14.3)
        let existing_jobs = load_jobs(&self.config.paths.job_state_dir).unwrap_or_else(|e| {
            eprintln!("Warning: Failed to load existing jobs: {}", e);
            Vec::new()
        });

        // Step 2: Scan all library_roots (Requirement 11.1)
        let candidates = scan_libraries(&self.config.scan.library_roots);

        // Create gates config from daemon config
        let gates_config = DaemonGatesConfig {
            min_bytes: self.config.gates.min_bytes,
            max_size_ratio: self.config.gates.max_size_ratio,
            keep_original: self.config.gates.keep_original,
        };

        // Step 3: Process each candidate
        for candidate in candidates {
            // Skip if job already exists for this path (Requirement 14.3)
            if job_exists_for_path(&existing_jobs, &candidate.path) {
                continue;
            }

            // Step 3a: Stability check (Requirements 12.1-12.4)
            let stability_result = match check_stability(
                &candidate.path,
                candidate.size_bytes,
                self.config.scan.stability_wait_secs,
            )
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    eprintln!(
                        "Warning: Stability check failed for {:?}: {}",
                        candidate.path, e
                    );
                    continue;
                }
            };

            // Skip unstable files (Requirement 12.3)
            if let StabilityResult::Unstable { .. } = stability_result {
                continue;
            }

            // Step 3b: Probe file (Requirement 13.1)
            let probe_result = match probe_file(&candidate.path) {
                Ok(result) => result,
                Err(e) => {
                    // Create skip marker on probe failure (Requirement 13.2)
                    let reason = format!("ffprobe failed: {}", e);
                    let _ = write_skip_marker(&candidate.path);
                    let _ = write_why_sidecar(
                        &candidate.path,
                        &reason,
                        self.config.scan.write_why_sidecars,
                    );
                    continue;
                }
            };

            // Step 3c: Check gates (Requirements 13.3-13.6)
            let gate_result = check_gates(&probe_result, candidate.size_bytes, &gates_config);

            match gate_result {
                GateResult::Skip { reason } => {
                    // Create skip markers (Requirements 13.3, 13.4, 13.5)
                    let _ = write_skip_marker(&candidate.path);
                    let _ = write_why_sidecar(
                        &candidate.path,
                        &reason,
                        self.config.scan.write_why_sidecars,
                    );
                    continue;
                }
                GateResult::Pass(probe) => {
                    // Step 3d: Classify source (Requirements 15.1-15.4)
                    let source_type = classify_source(&candidate.path, &probe);

                    // Step 3e: Create job (Requirement 14.1)
                    let managed_job = create_job(
                        &candidate,
                        probe.clone(),
                        source_type,
                        &self.config.paths.temp_output_dir,
                    );

                    // Save job to state directory (Requirement 14.2)
                    if let Err(e) = save_job(&managed_job, &self.config.paths.job_state_dir) {
                        eprintln!("Warning: Failed to save job state: {}", e);
                    }

                    // Step 4: Queue job for execution
                    let executor_job = Job::new(
                        managed_job.id.clone(),
                        managed_job.input_path.clone(),
                        managed_job.output_path.clone(),
                    );

                    // Set the original file size for size gate comparison
                    let mut job_with_size = executor_job;
                    job_with_size.size_in_bytes_before = candidate.size_bytes;

                    if let Err(e) = self.submit_job(job_with_size).await {
                        eprintln!("Warning: Failed to queue job: {}", e);
                        continue;
                    }

                    // Update queue length in metrics
                    {
                        let mut metrics = self.metrics.write().await;
                        metrics.queue_len += 1;
                    }

                    jobs_queued += 1;
                }
            }
        }

        Ok(jobs_queued)
    }

    /// Start the scan cycle task
    ///
    /// Periodically runs scan cycles to discover new files.
    ///
    /// # Requirements
    /// - 11.1: Recursively walk each configured library_root directory
    pub fn start_scan_cycle(&self) -> tokio::task::JoinHandle<()> {
        let config = self.config.clone();
        let job_tx = self.job_tx.clone();
        let metrics = self.metrics.clone();
        let job_state_dir = self.config.paths.job_state_dir.clone();
        let temp_output_dir = self.config.paths.temp_output_dir.clone();

        tokio::spawn(async move {
            loop {
                // Load existing jobs
                let existing_jobs = load_jobs(&job_state_dir).unwrap_or_else(|e| {
                    eprintln!("Warning: Failed to load existing jobs: {}", e);
                    Vec::new()
                });

                // Scan libraries
                let candidates = scan_libraries(&config.scan.library_roots);

                // Create gates config
                let gates_config = DaemonGatesConfig {
                    min_bytes: config.gates.min_bytes,
                    max_size_ratio: config.gates.max_size_ratio,
                    keep_original: config.gates.keep_original,
                };

                // Process candidates
                for candidate in candidates {
                    // Skip if job already exists
                    if job_exists_for_path(&existing_jobs, &candidate.path) {
                        continue;
                    }

                    // Stability check
                    let stability_result = match check_stability(
                        &candidate.path,
                        candidate.size_bytes,
                        config.scan.stability_wait_secs,
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => continue,
                    };

                    if let StabilityResult::Unstable { .. } = stability_result {
                        continue;
                    }

                    // Probe file
                    let probe_result = match probe_file(&candidate.path) {
                        Ok(result) => result,
                        Err(e) => {
                            let reason = format!("ffprobe failed: {}", e);
                            let _ = write_skip_marker(&candidate.path);
                            let _ = write_why_sidecar(
                                &candidate.path,
                                &reason,
                                config.scan.write_why_sidecars,
                            );
                            continue;
                        }
                    };

                    // Check gates
                    let gate_result =
                        check_gates(&probe_result, candidate.size_bytes, &gates_config);

                    match gate_result {
                        GateResult::Skip { reason } => {
                            let _ = write_skip_marker(&candidate.path);
                            let _ = write_why_sidecar(
                                &candidate.path,
                                &reason,
                                config.scan.write_why_sidecars,
                            );
                            continue;
                        }
                        GateResult::Pass(probe) => {
                            // Classify source
                            let source_type = classify_source(&candidate.path, &probe);

                            // Create job
                            let managed_job = create_job(
                                &candidate,
                                probe,
                                source_type,
                                &temp_output_dir,
                            );

                            // Save job state
                            if let Err(e) = save_job(&managed_job, &job_state_dir) {
                                eprintln!("Warning: Failed to save job state: {}", e);
                            }

                            // Create executor job
                            let mut executor_job = Job::new(
                                managed_job.id.clone(),
                                managed_job.input_path.clone(),
                                managed_job.output_path.clone(),
                            );
                            executor_job.size_in_bytes_before = candidate.size_bytes;

                            // Queue job
                            if job_tx.send(executor_job).await.is_ok() {
                                let mut m = metrics.write().await;
                                m.queue_len += 1;
                            }
                        }
                    }
                }

                // Wait before next scan cycle
                tokio::time::sleep(Duration::from_secs(config.scan.scan_interval_secs)).await;
            }
        })
    }

    /// Run the daemon with all background tasks
    ///
    /// Starts the metrics server, metrics updater, and main processing loop.
    pub async fn run_with_server(&self) -> Result<(), DaemonError> {
        // Start metrics server
        let _server_handle = self.start_metrics_server();

        // Start metrics updater
        let _updater_handle = self.start_metrics_updater();

        // Run main loop
        self.run().await
    }

    /// Run the daemon with all background tasks including scan cycle
    ///
    /// Starts the metrics server, metrics updater, scan cycle, and main processing loop.
    pub async fn run_with_scanning(&self) -> Result<(), DaemonError> {
        // Start metrics server
        let _server_handle = self.start_metrics_server();

        // Start metrics updater
        let _updater_handle = self.start_metrics_updater();

        // Start scan cycle
        let _scan_handle = self.start_scan_cycle();

        // Run main loop
        self.run().await
    }
}

/// Get current timestamp in milliseconds
fn chrono_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Av1anConfig, CpuConfig, EncoderSafetyConfig, GatesConfig, PathsConfig, ScanConfig};
    use tempfile::TempDir;

    fn create_test_config() -> Config {
        Config {
            cpu: CpuConfig {
                logical_cores: Some(32),
                target_cpu_utilization: 0.85,
            },
            av1an: Av1anConfig {
                workers_per_job: 8,
                max_concurrent_jobs: 1,
            },
            encoder_safety: EncoderSafetyConfig {
                disallow_hardware_encoding: true,
            },
            paths: PathsConfig::default(),
            scan: ScanConfig::default(),
            gates: GatesConfig::default(),
        }
    }

    fn create_test_config_with_paths(job_state_dir: PathBuf, temp_output_dir: PathBuf) -> Config {
        Config {
            cpu: CpuConfig {
                logical_cores: Some(32),
                target_cpu_utilization: 0.85,
            },
            av1an: Av1anConfig {
                workers_per_job: 8,
                max_concurrent_jobs: 1,
            },
            encoder_safety: EncoderSafetyConfig {
                disallow_hardware_encoding: true,
            },
            paths: PathsConfig {
                job_state_dir,
                temp_output_dir,
            },
            scan: ScanConfig::default(),
            gates: GatesConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_daemon_initialization_without_checks() {
        let config = create_test_config();
        let daemon = Daemon::new_without_checks(config.clone(), PathBuf::from("/tmp"));

        assert_eq!(daemon.config, config);
        assert_eq!(daemon.concurrency_plan.av1an_workers, 8);
        assert_eq!(daemon.concurrency_plan.max_concurrent_jobs, 1);
    }

    #[tokio::test]
    async fn test_daemon_derives_concurrency_plan() {
        let config = Config {
            cpu: CpuConfig {
                logical_cores: Some(48),
                target_cpu_utilization: 0.9,
            },
            av1an: Av1anConfig {
                workers_per_job: 0, // auto-derive
                max_concurrent_jobs: 0, // auto-derive
            },
            encoder_safety: EncoderSafetyConfig::default(),
            paths: PathsConfig::default(),
            scan: ScanConfig::default(),
            gates: GatesConfig::default(),
        };

        let daemon = Daemon::new_without_checks(config, PathBuf::from("/tmp"));

        // 48 cores >= 32, so workers should be 8
        assert_eq!(daemon.concurrency_plan.av1an_workers, 8);
        // 48 cores >= 24, so max_concurrent_jobs should be 1
        assert_eq!(daemon.concurrency_plan.max_concurrent_jobs, 1);
    }

    #[tokio::test]
    async fn test_daemon_job_submission() {
        let config = create_test_config();
        let daemon = Daemon::new_without_checks(config, PathBuf::from("/tmp"));

        let job = Job::new(
            "test-job-001".to_string(),
            PathBuf::from("/input/video.mkv"),
            PathBuf::from("/output/video.mkv"),
        );

        // Submit job should succeed
        let result = daemon.submit_job(job).await;
        assert!(result.is_ok());

        // Queue length should be updated
        let metrics = daemon.metrics.read().await;
        // Note: queue_len is managed by the run loop, not submit_job
        assert_eq!(metrics.queue_len, 0);
    }

    #[tokio::test]
    async fn test_daemon_metrics_initialized() {
        let config = create_test_config();
        let daemon = Daemon::new_without_checks(config, PathBuf::from("/tmp"));

        let metrics = daemon.metrics.read().await;
        assert_eq!(metrics.jobs.len(), 0);
        assert_eq!(metrics.running_jobs, 0);
        assert_eq!(metrics.completed_jobs, 0);
        assert_eq!(metrics.failed_jobs, 0);
    }

    #[test]
    fn test_chrono_timestamp_ms() {
        let ts = chrono_timestamp_ms();
        // Should be a reasonable timestamp (after year 2020)
        assert!(ts > 1577836800000); // Jan 1, 2020
    }

    #[test]
    fn test_create_required_directories_creates_both_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let job_state_dir = temp_dir.path().join("jobs");
        let temp_output_dir = temp_dir.path().join("temp");

        // Directories should not exist yet
        assert!(!job_state_dir.exists());
        assert!(!temp_output_dir.exists());

        let config = create_test_config_with_paths(job_state_dir.clone(), temp_output_dir.clone());

        // Create directories
        create_required_directories(&config).expect("Should create directories");

        // Both directories should now exist
        assert!(job_state_dir.exists());
        assert!(job_state_dir.is_dir());
        assert!(temp_output_dir.exists());
        assert!(temp_output_dir.is_dir());
    }

    #[test]
    fn test_create_required_directories_nested_paths() {
        let temp_dir = TempDir::new().unwrap();
        let job_state_dir = temp_dir.path().join("nested/path/to/jobs");
        let temp_output_dir = temp_dir.path().join("another/nested/temp");

        let config = create_test_config_with_paths(job_state_dir.clone(), temp_output_dir.clone());

        // Create directories (should create all parent directories)
        create_required_directories(&config).expect("Should create nested directories");

        assert!(job_state_dir.exists());
        assert!(temp_output_dir.exists());
    }

    #[test]
    fn test_create_required_directories_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let job_state_dir = temp_dir.path().join("jobs");
        let temp_output_dir = temp_dir.path().join("temp");

        let config = create_test_config_with_paths(job_state_dir.clone(), temp_output_dir.clone());

        // Create directories twice - should not fail
        create_required_directories(&config).expect("First call should succeed");
        create_required_directories(&config).expect("Second call should also succeed");

        assert!(job_state_dir.exists());
        assert!(temp_output_dir.exists());
    }

    #[test]
    fn test_create_required_directories_with_existing_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let job_state_dir = temp_dir.path().join("jobs");
        let temp_output_dir = temp_dir.path().join("temp");

        // Pre-create the directories
        fs::create_dir_all(&job_state_dir).unwrap();
        fs::create_dir_all(&temp_output_dir).unwrap();

        let config = create_test_config_with_paths(job_state_dir.clone(), temp_output_dir.clone());

        // Should succeed even if directories already exist
        create_required_directories(&config).expect("Should succeed with existing directories");

        assert!(job_state_dir.exists());
        assert!(temp_output_dir.exists());
    }
}
