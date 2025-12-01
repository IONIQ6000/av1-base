//! Daemon startup and main loop for AV1 Super Daemon
//!
//! Provides the daemon entry point, startup sequence, and main processing loop.

use crate::config::{Config, ConfigError};
use crate::concurrency::{derive_plan, ConcurrencyPlan};
use crate::job_executor::{Job, JobError, JobExecutor};
use crate::metrics::{collect_system_metrics, new_shared_metrics, SharedMetrics};
use crate::metrics_server::run_metrics_server;
use crate::startup::{run_startup_checks, StartupError};
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
    /// 4. Derive concurrency plan
    /// 5. Initialize shared metrics
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
    pub async fn new<P: AsRef<Path>>(
        config_path: P,
        temp_base_dir: PathBuf,
    ) -> Result<Self, DaemonError> {
        // Step 1 & 2: Load config from file and apply environment overrides
        let config = Config::load(config_path)?;

        // Step 3: Run startup checks in order: software-only, av1an, ffmpeg
        run_startup_checks(&config)?;

        // Step 4: Derive concurrency plan from configuration
        let concurrency_plan = derive_plan(&config);

        // Step 5: Initialize shared metrics
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
    use crate::config::{Av1anConfig, CpuConfig, EncoderSafetyConfig};

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
}
