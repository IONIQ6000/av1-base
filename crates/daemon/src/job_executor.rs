//! Job executor module for AV1 Super Daemon
//!
//! Manages the execution of encoding jobs with concurrency limiting via semaphore.

use crate::encode::{run_av1an, Av1anEncodeParams, EncodeError};
use crate::metrics::{JobMetrics, SharedMetrics};
use crate::replace::{atomic_replace, ReplaceError};
use crate::size_gate::{check_size_gate, SizeGateResult};
use crate::skip_marker::{write_skip_marker, write_why_sidecar};
use crate::ConcurrencyPlan;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Error type for job execution operations
#[derive(Debug, Error)]
pub enum JobError {
    /// Encoding failed
    #[error("Encode failed: {0}")]
    Encode(#[from] EncodeError),

    /// Failed to create temp directory
    #[error("Failed to create temp directory: {0}")]
    TempDirCreation(std::io::Error),

    /// Validation failed
    #[error("Validation failed: {0}")]
    Validation(String),

    /// File replacement failed
    #[error("Replacement failed: {0}")]
    Replacement(#[from] ReplaceError),

    /// Size gate rejected the encode
    #[error("Size gate rejected: output {output_bytes} >= original {original_bytes} * {ratio}")]
    SizeGateRejected {
        original_bytes: u64,
        output_bytes: u64,
        ratio: f32,
    },

    /// Failed to write skip marker
    #[error("Failed to write skip marker: {0}")]
    SkipMarkerFailed(std::io::Error),
}

/// Job state representing the current stage in the pipeline
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    /// Job is waiting in queue
    Queued,
    /// Job is currently encoding
    Encoding,
    /// Job is being validated
    Validating,
    /// Job is going through size gate check
    SizeGating,
    /// Job is replacing original file
    Replacing,
    /// Job completed successfully
    Completed,
    /// Job was skipped (e.g., size gate rejection)
    Skipped(String),
    /// Job failed
    Failed(String),
}

impl JobState {
    /// Convert state to string for metrics
    pub fn as_str(&self) -> &str {
        match self {
            JobState::Queued => "queued",
            JobState::Encoding => "encoding",
            JobState::Validating => "validating",
            JobState::SizeGating => "size_gating",
            JobState::Replacing => "replacing",
            JobState::Completed => "completed",
            JobState::Skipped(_) => "skipped",
            JobState::Failed(_) => "failed",
        }
    }
}


/// Represents an encoding job to be executed
#[derive(Debug, Clone)]
pub struct Job {
    /// Unique job identifier
    pub id: String,
    /// Path to the input video file
    pub input_path: PathBuf,
    /// Path for the encoded output file
    pub output_path: PathBuf,
    /// Current state of the job
    pub state: JobState,
    /// Total frames in the video (if known)
    pub total_frames: u64,
    /// Original file size in bytes
    pub size_in_bytes_before: u64,
}

impl Job {
    /// Create a new job
    pub fn new(id: String, input_path: PathBuf, output_path: PathBuf) -> Self {
        Self {
            id,
            input_path,
            output_path,
            state: JobState::Queued,
            total_frames: 0,
            size_in_bytes_before: 0,
        }
    }

    /// Create JobMetrics from current job state
    pub fn to_metrics(&self, workers: u32) -> JobMetrics {
        JobMetrics {
            id: self.id.clone(),
            input_path: self.input_path.to_string_lossy().to_string(),
            stage: self.state.as_str().to_string(),
            progress: 0.0,
            fps: 0.0,
            bitrate_kbps: 0.0,
            crf: 8,
            encoder: "svt-av1".to_string(),
            workers,
            est_remaining_secs: 0.0,
            frames_encoded: 0,
            total_frames: self.total_frames,
            size_in_bytes_before: self.size_in_bytes_before,
            size_in_bytes_after: 0,
            vmaf: None,
            psnr: None,
            ssim: None,
        }
    }
}

/// Configuration for the job executor pipeline
#[derive(Debug, Clone)]
pub struct JobExecutorConfig {
    /// Maximum size ratio for size gate (output/original, e.g., 0.95)
    pub max_size_ratio: f32,
    /// Whether to keep the original file backup after replacement
    pub keep_original: bool,
    /// Whether to write .why.txt sidecar files explaining skips
    pub write_why_sidecars: bool,
}

impl Default for JobExecutorConfig {
    fn default() -> Self {
        Self {
            max_size_ratio: 0.95,
            keep_original: false,
            write_why_sidecars: true,
        }
    }
}

/// Job executor that manages encoding job execution with concurrency limiting
///
/// Uses a tokio Semaphore to limit the number of concurrent encoding jobs
/// according to the concurrency plan.
pub struct JobExecutor {
    /// Semaphore for limiting concurrent jobs
    semaphore: Arc<Semaphore>,
    /// Concurrency plan with worker and job limits
    concurrency_plan: ConcurrencyPlan,
    /// Shared metrics state
    metrics: SharedMetrics,
    /// Base directory for temporary chunk files
    temp_base_dir: PathBuf,
    /// Configuration for the pipeline
    config: JobExecutorConfig,
}

impl JobExecutor {
    /// Create a new JobExecutor
    ///
    /// # Arguments
    /// * `plan` - Concurrency plan determining max concurrent jobs
    /// * `metrics` - Shared metrics state for updating job progress
    /// * `temp_base_dir` - Base directory for creating temporary chunk directories
    pub fn new(plan: ConcurrencyPlan, metrics: SharedMetrics, temp_base_dir: PathBuf) -> Self {
        let permits = plan.max_concurrent_jobs as usize;
        Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            concurrency_plan: plan,
            metrics,
            temp_base_dir,
            config: JobExecutorConfig::default(),
        }
    }

    /// Create a new JobExecutor with custom configuration
    ///
    /// # Arguments
    /// * `plan` - Concurrency plan determining max concurrent jobs
    /// * `metrics` - Shared metrics state for updating job progress
    /// * `temp_base_dir` - Base directory for creating temporary chunk directories
    /// * `config` - Configuration for the pipeline
    pub fn with_config(
        plan: ConcurrencyPlan,
        metrics: SharedMetrics,
        temp_base_dir: PathBuf,
        config: JobExecutorConfig,
    ) -> Self {
        let permits = plan.max_concurrent_jobs as usize;
        Self {
            semaphore: Arc::new(Semaphore::new(permits)),
            concurrency_plan: plan,
            metrics,
            temp_base_dir,
            config,
        }
    }

    /// Get the number of available permits (slots for concurrent jobs)
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the concurrency plan
    pub fn concurrency_plan(&self) -> &ConcurrencyPlan {
        &self.concurrency_plan
    }

    /// Acquire a permit for job execution
    ///
    /// This will wait until a permit is available if all slots are in use.
    pub async fn acquire_permit(&self) -> OwnedSemaphorePermit {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore should not be closed")
    }

    /// Try to acquire a permit without waiting
    ///
    /// Returns None if no permits are available.
    pub fn try_acquire_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }


    /// Execute a job through the encoding pipeline
    ///
    /// This method implements the full encoding pipeline:
    /// 1. Acquires a semaphore permit (respecting max_concurrent_jobs)
    /// 2. Creates a temporary chunks directory (Requirement 5.1)
    /// 3. Runs Av1an encoding (Requirements 5.2, 5.3)
    /// 4. Validates the output file
    /// 5. Runs size gate check (Requirements 16.1, 16.2, 16.3, 16.4)
    /// 6. Performs atomic file replacement (Requirements 17.1-17.6)
    /// 7. Creates skip markers on size gate failure (Requirements 18.1, 18.2)
    /// 8. Updates job state at each stage
    ///
    /// # Arguments
    /// * `job` - The job to execute
    ///
    /// # Returns
    /// * `Ok(Job)` - Job completed successfully with updated state
    /// * `Err(JobError)` - Job failed with error details
    pub async fn execute(&self, mut job: Job) -> Result<Job, JobError> {
        // Acquire permit to respect max_concurrent_jobs limit (Requirement 5.5)
        let _permit = self.acquire_permit().await;

        // Update job state to encoding
        job.state = JobState::Encoding;
        self.update_job_metrics(&job).await;

        // Create temp chunks directory (Requirement 5.1)
        let temp_chunks_dir = self.temp_base_dir.join(format!("chunks_{}", job.id));
        std::fs::create_dir_all(&temp_chunks_dir).map_err(JobError::TempDirCreation)?;

        // Build encoding parameters
        let params = Av1anEncodeParams::new(
            job.input_path.clone(),
            job.output_path.clone(),
            temp_chunks_dir.clone(),
            self.concurrency_plan.clone(),
        );

        // Run Av1an encoding (Requirements 5.2, 5.3)
        let encode_result = tokio::task::spawn_blocking(move || run_av1an(&params)).await;

        match encode_result {
            Ok(Ok(())) => {
                // Encoding succeeded, proceed to validation (Requirement 5.2)
                job.state = JobState::Validating;
                self.update_job_metrics(&job).await;

                // Validate the output file exists and has content
                let output_metadata = match std::fs::metadata(&job.output_path) {
                    Ok(m) => m,
                    Err(e) => {
                        let error_msg = format!("Output file not found: {}", e);
                        job.state = JobState::Failed(error_msg.clone());
                        self.update_job_metrics(&job).await;
                        self.increment_failed_jobs().await;
                        let _ = std::fs::remove_dir_all(&temp_chunks_dir);
                        return Err(JobError::Validation(error_msg));
                    }
                };

                let output_bytes = output_metadata.len();
                if output_bytes == 0 {
                    let error_msg = "Output file is empty".to_string();
                    job.state = JobState::Failed(error_msg.clone());
                    self.update_job_metrics(&job).await;
                    self.increment_failed_jobs().await;
                    let _ = std::fs::remove_dir_all(&temp_chunks_dir);
                    let _ = std::fs::remove_file(&job.output_path);
                    return Err(JobError::Validation(error_msg));
                }

                // Size gate check (Requirements 16.1, 16.2, 16.3, 16.4)
                job.state = JobState::SizeGating;
                self.update_job_metrics(&job).await;

                let size_gate_result = check_size_gate(
                    job.size_in_bytes_before,
                    output_bytes,
                    self.config.max_size_ratio,
                );

                match size_gate_result {
                    SizeGateResult::Accept => {
                        // Size gate passed, proceed to replacement
                        job.state = JobState::Replacing;
                        self.update_job_metrics(&job).await;

                        // Atomic file replacement (Requirements 17.1-17.6)
                        match atomic_replace(
                            &job.input_path,
                            &job.output_path,
                            self.config.keep_original,
                        ) {
                            Ok(()) => {
                                // Mark as completed (Requirement 5.4)
                                job.state = JobState::Completed;
                                self.update_job_metrics(&job).await;
                                self.increment_completed_jobs().await;

                                // Update size_in_bytes_after for metrics
                                self.update_job_size_after(&job.id, output_bytes).await;

                                // Clean up temp directory and output file
                                let _ = std::fs::remove_dir_all(&temp_chunks_dir);
                                let _ = std::fs::remove_file(&job.output_path);

                                Ok(job)
                            }
                            Err(replace_err) => {
                                // Replacement failed (Requirement 17.6)
                                let error_msg = replace_err.to_string();
                                job.state = JobState::Failed(error_msg);
                                self.update_job_metrics(&job).await;
                                self.increment_failed_jobs().await;

                                // Preserve temp files for manual inspection
                                // Don't clean up temp_chunks_dir or output_path

                                Err(JobError::Replacement(replace_err))
                            }
                        }
                    }
                    SizeGateResult::Reject {
                        original_bytes,
                        output_bytes,
                        ratio,
                    } => {
                        // Size gate rejected (Requirement 16.3)
                        let skip_reason = format!(
                            "Size gate rejected: output {} bytes ({:.1}%) >= original {} bytes * {:.2}",
                            output_bytes,
                            ratio * 100.0,
                            original_bytes,
                            self.config.max_size_ratio
                        );

                        job.state = JobState::Skipped(skip_reason.clone());
                        self.update_job_metrics(&job).await;
                        self.increment_skipped_jobs().await;

                        // Delete temp output (Requirement 16.3)
                        let _ = std::fs::remove_file(&job.output_path);

                        // Create skip markers (Requirements 18.1, 18.2)
                        write_skip_marker(&job.input_path)
                            .map_err(JobError::SkipMarkerFailed)?;
                        
                        // Write why sidecar if enabled
                        let _ = write_why_sidecar(
                            &job.input_path,
                            &skip_reason,
                            self.config.write_why_sidecars,
                        );

                        // Clean up temp directory
                        let _ = std::fs::remove_dir_all(&temp_chunks_dir);

                        Err(JobError::SizeGateRejected {
                            original_bytes,
                            output_bytes,
                            ratio,
                        })
                    }
                }
            }
            Ok(Err(encode_err)) => {
                // Encoding failed (Requirement 5.3)
                job.state = JobState::Failed(encode_err.to_string());
                self.update_job_metrics(&job).await;
                self.increment_failed_jobs().await;

                // Clean up temp directory
                let _ = std::fs::remove_dir_all(&temp_chunks_dir);

                Err(JobError::Encode(encode_err))
            }
            Err(join_err) => {
                // Task panicked
                let error_msg = format!("Encoding task panicked: {}", join_err);
                job.state = JobState::Failed(error_msg.clone());
                self.update_job_metrics(&job).await;
                self.increment_failed_jobs().await;

                // Clean up temp directory
                let _ = std::fs::remove_dir_all(&temp_chunks_dir);

                Err(JobError::Validation(error_msg))
            }
        }
    }

    /// Update job metrics in shared state
    async fn update_job_metrics(&self, job: &Job) {
        let mut metrics = self.metrics.write().await;
        let job_metrics = job.to_metrics(self.concurrency_plan.av1an_workers);

        // Find and update existing job metrics, or add new one
        if let Some(existing) = metrics.jobs.iter_mut().find(|j| j.id == job.id) {
            *existing = job_metrics;
        } else {
            metrics.jobs.push(job_metrics);
        }

        // Update running jobs count
        metrics.running_jobs = metrics
            .jobs
            .iter()
            .filter(|j| j.stage == "encoding" || j.stage == "validating" || j.stage == "replacing")
            .count();
    }

    /// Increment completed jobs counter
    async fn increment_completed_jobs(&self) {
        let mut metrics = self.metrics.write().await;
        metrics.completed_jobs += 1;
    }

    /// Increment failed jobs counter
    async fn increment_failed_jobs(&self) {
        let mut metrics = self.metrics.write().await;
        metrics.failed_jobs += 1;
    }

    /// Increment skipped jobs counter (for size gate rejections)
    async fn increment_skipped_jobs(&self) {
        let mut metrics = self.metrics.write().await;
        // Skipped jobs are counted as failed in the aggregate metrics
        metrics.failed_jobs += 1;
    }

    /// Update the size_in_bytes_after for a completed job
    async fn update_job_size_after(&self, job_id: &str, size_bytes: u64) {
        let mut metrics = self.metrics.write().await;
        if let Some(job_metrics) = metrics.jobs.iter_mut().find(|j| j.id == job_id) {
            job_metrics.size_in_bytes_after = size_bytes;
        }
        metrics.total_bytes_encoded += size_bytes;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::new_shared_metrics;
    use std::time::Duration;

    fn create_test_plan(max_concurrent_jobs: u32) -> ConcurrencyPlan {
        ConcurrencyPlan {
            total_cores: 32,
            target_threads: 28,
            av1an_workers: 8,
            max_concurrent_jobs,
        }
    }

    fn create_test_job(id: &str) -> Job {
        Job::new(
            id.to_string(),
            PathBuf::from("/tmp/input.mkv"),
            PathBuf::from("/tmp/output.mkv"),
        )
    }

    // Test that JobExecutor initializes with correct number of permits
    // **Validates: Requirements 5.5**
    #[tokio::test]
    async fn test_executor_initial_permits() {
        let plan = create_test_plan(3);
        let metrics = new_shared_metrics();
        let executor = JobExecutor::new(plan, metrics, PathBuf::from("/tmp"));

        assert_eq!(executor.available_permits(), 3);
    }

    // Test that semaphore correctly limits concurrent jobs
    // **Validates: Requirements 5.5**
    #[tokio::test]
    async fn test_semaphore_permit_limiting() {
        let plan = create_test_plan(2);
        let metrics = new_shared_metrics();
        let executor = JobExecutor::new(plan, metrics, PathBuf::from("/tmp"));

        // Initially should have 2 permits
        assert_eq!(executor.available_permits(), 2);

        // Acquire first permit
        let permit1 = executor.try_acquire_permit();
        assert!(permit1.is_some());
        assert_eq!(executor.available_permits(), 1);

        // Acquire second permit
        let permit2 = executor.try_acquire_permit();
        assert!(permit2.is_some());
        assert_eq!(executor.available_permits(), 0);

        // Third acquire should fail (no permits available)
        let permit3 = executor.try_acquire_permit();
        assert!(permit3.is_none());

        // Drop first permit, should have 1 available again
        drop(permit1);
        assert_eq!(executor.available_permits(), 1);

        // Now we can acquire again
        let permit4 = executor.try_acquire_permit();
        assert!(permit4.is_some());
        assert_eq!(executor.available_permits(), 0);
    }

    // Test job state transitions
    // **Validates: Requirements 5.1, 5.2, 5.3, 5.4, 16.3**
    #[test]
    fn test_job_state_as_str() {
        assert_eq!(JobState::Queued.as_str(), "queued");
        assert_eq!(JobState::Encoding.as_str(), "encoding");
        assert_eq!(JobState::Validating.as_str(), "validating");
        assert_eq!(JobState::SizeGating.as_str(), "size_gating");
        assert_eq!(JobState::Replacing.as_str(), "replacing");
        assert_eq!(JobState::Completed.as_str(), "completed");
        assert_eq!(JobState::Skipped("reason".to_string()).as_str(), "skipped");
        assert_eq!(JobState::Failed("error".to_string()).as_str(), "failed");
    }

    // Test job creation and initial state
    #[test]
    fn test_job_creation() {
        let job = create_test_job("test-001");

        assert_eq!(job.id, "test-001");
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(job.total_frames, 0);
        assert_eq!(job.size_in_bytes_before, 0);
    }

    // Test job to metrics conversion
    #[test]
    fn test_job_to_metrics() {
        let mut job = create_test_job("test-002");
        job.state = JobState::Encoding;
        job.total_frames = 120000;
        job.size_in_bytes_before = 5368709120;

        let metrics = job.to_metrics(8);

        assert_eq!(metrics.id, "test-002");
        assert_eq!(metrics.stage, "encoding");
        assert_eq!(metrics.workers, 8);
        assert_eq!(metrics.total_frames, 120000);
        assert_eq!(metrics.size_in_bytes_before, 5368709120);
        assert_eq!(metrics.encoder, "svt-av1");
        assert_eq!(metrics.crf, 8);
    }

    // Test that metrics are updated during job execution
    // **Validates: Requirements 5.5**
    #[tokio::test]
    async fn test_metrics_update_on_state_change() {
        let plan = create_test_plan(1);
        let metrics = new_shared_metrics();
        let executor = JobExecutor::new(plan, metrics.clone(), PathBuf::from("/tmp"));

        let job = create_test_job("metrics-test");

        // Manually update metrics as if job started
        executor.update_job_metrics(&job).await;

        // Check metrics were updated
        let snapshot = metrics.read().await;
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.jobs[0].id, "metrics-test");
        assert_eq!(snapshot.jobs[0].stage, "queued");
    }

    // Test JobExecutorConfig defaults
    #[test]
    fn test_job_executor_config_defaults() {
        let config = JobExecutorConfig::default();
        assert!((config.max_size_ratio - 0.95).abs() < 0.001);
        assert!(!config.keep_original);
        assert!(config.write_why_sidecars);
    }

    // Test JobExecutor with custom config
    #[tokio::test]
    async fn test_executor_with_custom_config() {
        let plan = create_test_plan(2);
        let metrics = new_shared_metrics();
        let config = JobExecutorConfig {
            max_size_ratio: 0.80,
            keep_original: true,
            write_why_sidecars: false,
        };
        let executor = JobExecutor::with_config(
            plan,
            metrics,
            PathBuf::from("/tmp"),
            config,
        );

        assert_eq!(executor.available_permits(), 2);
        assert!((executor.config.max_size_ratio - 0.80).abs() < 0.001);
        assert!(executor.config.keep_original);
        assert!(!executor.config.write_why_sidecars);
    }

    // Test concurrent permit acquisition with async tasks
    // **Validates: Requirements 5.5**
    #[tokio::test]
    async fn test_concurrent_permit_acquisition() {
        let plan = create_test_plan(2);
        let metrics = new_shared_metrics();
        let executor = Arc::new(JobExecutor::new(plan, metrics, PathBuf::from("/tmp")));

        let executor1 = executor.clone();
        let executor2 = executor.clone();
        let executor3 = executor.clone();

        // Spawn three tasks trying to acquire permits
        let handle1 = tokio::spawn(async move {
            let _permit = executor1.acquire_permit().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        let handle2 = tokio::spawn(async move {
            let _permit = executor2.acquire_permit().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        // Give first two tasks time to acquire permits
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Third task should have to wait
        let start = std::time::Instant::now();
        let handle3 = tokio::spawn(async move {
            let _permit = executor3.acquire_permit().await;
        });

        // Wait for all tasks
        let _ = tokio::join!(handle1, handle2, handle3);

        // Third task should have waited at least ~90ms for a permit
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(50));
    }
}
