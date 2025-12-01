//! Job manager module for persisting and managing encoding jobs.
//!
//! This module provides functionality to create, save, load, and query jobs.
//! Jobs are persisted as JSON files in a configured state directory.

use crate::classify::SourceType;
use crate::gates::ProbeResult;
use crate::scan::ScanCandidate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Stage of a job in the encoding pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    /// Job is waiting in queue.
    Queued,
    /// Job is currently encoding.
    Encoding,
    /// Job is being validated after encoding.
    Validating,
    /// Job is going through size gate check.
    SizeGating,
    /// Job is replacing the original file.
    Replacing,
    /// Job has completed successfully.
    Complete,
}

impl Default for JobStage {
    fn default() -> Self {
        Self::Queued
    }
}

impl std::fmt::Display for JobStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStage::Queued => write!(f, "queued"),
            JobStage::Encoding => write!(f, "encoding"),
            JobStage::Validating => write!(f, "validating"),
            JobStage::SizeGating => write!(f, "size_gating"),
            JobStage::Replacing => write!(f, "replacing"),
            JobStage::Complete => write!(f, "complete"),
        }
    }
}


/// Status of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Job is pending execution.
    Pending,
    /// Job is currently running.
    Running,
    /// Job completed successfully.
    Success,
    /// Job failed with an error.
    Failed,
    /// Job was skipped (e.g., size gate rejection).
    Skipped,
}

impl Default for JobStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Success => write!(f, "success"),
            JobStatus::Failed => write!(f, "failed"),
            JobStatus::Skipped => write!(f, "skipped"),
        }
    }
}

/// Represents an encoding job with full metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    /// Unique job identifier (UUID).
    pub id: String,
    /// Path to the input video file.
    pub input_path: PathBuf,
    /// Path for the encoded output file.
    pub output_path: PathBuf,
    /// Current stage in the pipeline.
    pub stage: JobStage,
    /// Current status of the job.
    pub status: JobStatus,
    /// Classification of the source (web-like, disc-like, unknown).
    pub source_type: SourceType,
    /// Probe result from ffprobe.
    pub probe_result: ProbeResult,
    /// Unix timestamp (milliseconds) when job was created.
    pub created_at: i64,
    /// Unix timestamp (milliseconds) when job was last updated.
    pub updated_at: i64,
    /// Error reason if job failed or was skipped.
    pub error_reason: Option<String>,
}

impl Job {
    /// Update the job's updated_at timestamp to now.
    pub fn touch(&mut self) {
        self.updated_at = current_timestamp_ms();
    }

    /// Set the job stage and update timestamp.
    pub fn set_stage(&mut self, stage: JobStage) {
        self.stage = stage;
        self.touch();
    }

    /// Set the job status and update timestamp.
    pub fn set_status(&mut self, status: JobStatus) {
        self.status = status;
        self.touch();
    }

    /// Mark the job as failed with a reason.
    pub fn fail(&mut self, reason: &str) {
        self.status = JobStatus::Failed;
        self.error_reason = Some(reason.to_string());
        self.touch();
    }

    /// Mark the job as skipped with a reason.
    pub fn skip(&mut self, reason: &str) {
        self.status = JobStatus::Skipped;
        self.error_reason = Some(reason.to_string());
        self.touch();
    }

    /// Check if the job is in a terminal state (success, failed, or skipped).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            JobStatus::Success | JobStatus::Failed | JobStatus::Skipped
        )
    }

    /// Check if the job is active (pending or running).
    pub fn is_active(&self) -> bool {
        matches!(self.status, JobStatus::Pending | JobStatus::Running)
    }
}


/// Get current timestamp in milliseconds since Unix epoch.
fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Creates a new job from a scan candidate, probe result, and source type.
///
/// Generates a UUID for the job id, sets initial stage to Queued and status to Pending.
///
/// # Arguments
/// * `candidate` - The scan candidate containing input path and file info
/// * `probe_result` - The ffprobe result for the file
/// * `source_type` - The classified source type
/// * `temp_output_dir` - Base directory for temporary output files
pub fn create_job(
    candidate: &ScanCandidate,
    probe_result: ProbeResult,
    source_type: SourceType,
    temp_output_dir: &Path,
) -> Job {
    let id = Uuid::new_v4().to_string();
    let now = current_timestamp_ms();

    // Generate output path in temp directory
    let output_filename = format!("{}.mkv", id);
    let output_path = temp_output_dir.join(output_filename);

    Job {
        id,
        input_path: candidate.path.clone(),
        output_path,
        stage: JobStage::Queued,
        status: JobStatus::Pending,
        source_type,
        probe_result,
        created_at: now,
        updated_at: now,
        error_reason: None,
    }
}

/// Saves a job to a JSON file in the state directory.
///
/// The file is named `{job_id}.json`.
///
/// # Arguments
/// * `job` - The job to save
/// * `state_dir` - Directory where job JSON files are stored
pub fn save_job(job: &Job, state_dir: &Path) -> Result<(), io::Error> {
    // Ensure state directory exists
    fs::create_dir_all(state_dir)?;

    let file_path = state_dir.join(format!("{}.json", job.id));
    let json = serde_json::to_string_pretty(job)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    fs::write(file_path, json)
}

/// Loads all jobs from JSON files in the state directory.
///
/// Skips files that fail to parse and logs warnings.
///
/// # Arguments
/// * `state_dir` - Directory where job JSON files are stored
pub fn load_jobs(state_dir: &Path) -> Result<Vec<Job>, io::Error> {
    if !state_dir.exists() {
        return Ok(Vec::new());
    }

    let mut jobs = Vec::new();

    for entry in fs::read_dir(state_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Only process .json files
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        match load_job_from_file(&path) {
            Ok(job) => jobs.push(job),
            Err(e) => {
                // Log warning but continue loading other jobs
                eprintln!("Warning: Failed to load job from {:?}: {}", path, e);
            }
        }
    }

    Ok(jobs)
}

/// Loads a single job from a JSON file.
fn load_job_from_file(path: &Path) -> Result<Job, io::Error> {
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Checks if a job already exists for the given input path.
///
/// Returns true if any pending or running job exists for the path.
///
/// # Arguments
/// * `jobs` - List of existing jobs to check
/// * `path` - Input path to check for
pub fn job_exists_for_path(jobs: &[Job], path: &Path) -> bool {
    jobs.iter().any(|job| {
        job.input_path == path && job.is_active()
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::{AudioStream, FormatInfo, VideoStream};
    use proptest::prelude::*;
    use tempfile::TempDir;

    /// Helper to create a VideoStream for testing.
    fn make_video_stream(codec: &str, width: u32, height: u32) -> VideoStream {
        VideoStream {
            codec_name: codec.to_string(),
            width,
            height,
            bitrate_kbps: Some(5000.0),
        }
    }

    /// Helper to create an AudioStream for testing.
    fn make_audio_stream(codec: &str, channels: u32) -> AudioStream {
        AudioStream {
            codec_name: codec.to_string(),
            channels,
        }
    }

    /// Helper to create a ProbeResult for testing.
    fn make_probe_result() -> ProbeResult {
        ProbeResult {
            video_streams: vec![make_video_stream("hevc", 1920, 1080)],
            audio_streams: vec![make_audio_stream("aac", 6)],
            format: FormatInfo {
                duration_secs: 7200.0,
                size_bytes: 22548578304,
            },
        }
    }

    /// Helper to create a ScanCandidate for testing.
    fn make_scan_candidate(path: &str) -> ScanCandidate {
        ScanCandidate {
            path: PathBuf::from(path),
            size_bytes: 5_000_000_000,
            modified_time: SystemTime::now(),
        }
    }

    // Strategy for generating arbitrary source types
    fn source_type_strategy() -> impl Strategy<Value = SourceType> {
        prop_oneof![
            Just(SourceType::WebLike),
            Just(SourceType::DiscLike),
            Just(SourceType::Unknown),
        ]
    }

    // Strategy for generating arbitrary job stages
    fn job_stage_strategy() -> impl Strategy<Value = JobStage> {
        prop_oneof![
            Just(JobStage::Queued),
            Just(JobStage::Encoding),
            Just(JobStage::Validating),
            Just(JobStage::SizeGating),
            Just(JobStage::Replacing),
            Just(JobStage::Complete),
        ]
    }

    // Strategy for generating arbitrary job statuses
    fn job_status_strategy() -> impl Strategy<Value = JobStatus> {
        prop_oneof![
            Just(JobStatus::Pending),
            Just(JobStatus::Running),
            Just(JobStatus::Success),
            Just(JobStatus::Failed),
            Just(JobStatus::Skipped),
        ]
    }

    // Strategy for generating video streams
    fn video_stream_strategy() -> impl Strategy<Value = VideoStream> {
        (
            "[a-z0-9]{2,10}",
            1u32..8000,
            1u32..4500,
            prop::option::of(1.0f32..100000.0),
        )
            .prop_map(|(codec, width, height, bitrate)| VideoStream {
                codec_name: codec,
                width,
                height,
                bitrate_kbps: bitrate,
            })
    }

    // Strategy for generating audio streams
    fn audio_stream_strategy() -> impl Strategy<Value = AudioStream> {
        ("[a-z0-9]{2,10}", 1u32..16).prop_map(|(codec, channels)| AudioStream {
            codec_name: codec,
            channels,
        })
    }

    // Strategy for generating probe results
    fn probe_result_strategy() -> impl Strategy<Value = ProbeResult> {
        (
            prop::collection::vec(video_stream_strategy(), 0..3),
            prop::collection::vec(audio_stream_strategy(), 0..5),
            0.0f64..100000.0,
            0u64..100_000_000_000,
        )
            .prop_map(|(video_streams, audio_streams, duration, size)| ProbeResult {
                video_streams,
                audio_streams,
                format: FormatInfo {
                    duration_secs: duration,
                    size_bytes: size,
                },
            })
    }

    // Strategy for generating jobs
    fn job_strategy() -> impl Strategy<Value = Job> {
        (
            "[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}",
            "[a-zA-Z0-9/_.-]{5,50}",
            "[a-zA-Z0-9/_.-]{5,50}",
            job_stage_strategy(),
            job_status_strategy(),
            source_type_strategy(),
            probe_result_strategy(),
            0i64..2_000_000_000_000i64,
            0i64..2_000_000_000_000i64,
            prop::option::of("[a-zA-Z0-9 ]{0,100}"),
        )
            .prop_map(
                |(id, input, output, stage, status, source_type, probe, created, updated, error)| {
                    Job {
                        id,
                        input_path: PathBuf::from(input),
                        output_path: PathBuf::from(output),
                        stage,
                        status,
                        source_type,
                        probe_result: probe,
                        created_at: created,
                        updated_at: updated,
                        error_reason: error,
                    }
                },
            )
    }

    // **Feature: av1-super-daemon, Property 17: Job JSON Serialization Round-Trip**
    // **Validates: Requirements 14.1, 14.2, 14.4**
    //
    // *For any* valid `Job` struct, serializing to JSON and deserializing back SHALL
    // produce an equivalent job with all fields preserved (id, paths, stage, status,
    // source_type, probe_result, timestamps, error_reason).
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_job_json_round_trip(job in job_strategy()) {
            // Serialize to JSON
            let json = serde_json::to_string(&job)
                .expect("Job should serialize to JSON");

            // Deserialize back
            let deserialized: Job = serde_json::from_str(&json)
                .expect("JSON should deserialize back to Job");

            // All fields should be preserved
            prop_assert_eq!(&job.id, &deserialized.id, "id mismatch");
            prop_assert_eq!(&job.input_path, &deserialized.input_path, "input_path mismatch");
            prop_assert_eq!(&job.output_path, &deserialized.output_path, "output_path mismatch");
            prop_assert_eq!(job.stage, deserialized.stage, "stage mismatch");
            prop_assert_eq!(job.status, deserialized.status, "status mismatch");
            prop_assert_eq!(job.source_type, deserialized.source_type, "source_type mismatch");
            prop_assert_eq!(job.created_at, deserialized.created_at, "created_at mismatch");
            prop_assert_eq!(job.updated_at, deserialized.updated_at, "updated_at mismatch");
            prop_assert_eq!(&job.error_reason, &deserialized.error_reason, "error_reason mismatch");

            // Probe result should match
            prop_assert_eq!(
                job.probe_result.video_streams.len(),
                deserialized.probe_result.video_streams.len(),
                "video_streams count mismatch"
            );
            prop_assert_eq!(
                job.probe_result.audio_streams.len(),
                deserialized.probe_result.audio_streams.len(),
                "audio_streams count mismatch"
            );
            prop_assert_eq!(
                job.probe_result.format.size_bytes,
                deserialized.probe_result.format.size_bytes,
                "format.size_bytes mismatch"
            );
        }
    }


    // Unit tests

    #[test]
    fn test_job_stage_display() {
        assert_eq!(format!("{}", JobStage::Queued), "queued");
        assert_eq!(format!("{}", JobStage::Encoding), "encoding");
        assert_eq!(format!("{}", JobStage::Validating), "validating");
        assert_eq!(format!("{}", JobStage::SizeGating), "size_gating");
        assert_eq!(format!("{}", JobStage::Replacing), "replacing");
        assert_eq!(format!("{}", JobStage::Complete), "complete");
    }

    #[test]
    fn test_job_status_display() {
        assert_eq!(format!("{}", JobStatus::Pending), "pending");
        assert_eq!(format!("{}", JobStatus::Running), "running");
        assert_eq!(format!("{}", JobStatus::Success), "success");
        assert_eq!(format!("{}", JobStatus::Failed), "failed");
        assert_eq!(format!("{}", JobStatus::Skipped), "skipped");
    }

    #[test]
    fn test_job_stage_default() {
        assert_eq!(JobStage::default(), JobStage::Queued);
    }

    #[test]
    fn test_job_status_default() {
        assert_eq!(JobStatus::default(), JobStatus::Pending);
    }

    #[test]
    fn test_create_job() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let job = create_job(&candidate, probe.clone(), SourceType::DiscLike, &temp_dir);

        // Check UUID format (36 chars with hyphens)
        assert_eq!(job.id.len(), 36);
        assert!(job.id.contains('-'));

        // Check initial state
        assert_eq!(job.stage, JobStage::Queued);
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.source_type, SourceType::DiscLike);
        assert_eq!(job.input_path, PathBuf::from("/media/movies/film.mkv"));
        assert!(job.output_path.starts_with(&temp_dir));
        assert!(job.output_path.to_string_lossy().ends_with(".mkv"));
        assert!(job.created_at > 0);
        assert_eq!(job.created_at, job.updated_at);
        assert!(job.error_reason.is_none());

        // Check probe result is stored
        assert_eq!(job.probe_result.video_streams.len(), 1);
        assert_eq!(job.probe_result.video_streams[0].codec_name, "hevc");
    }

    #[test]
    fn test_job_touch() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::WebLike, &temp_dir);
        let original_updated = job.updated_at;

        // Small delay to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        job.touch();

        assert!(job.updated_at >= original_updated);
    }

    #[test]
    fn test_job_set_stage() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::Unknown, &temp_dir);

        job.set_stage(JobStage::Encoding);
        assert_eq!(job.stage, JobStage::Encoding);

        job.set_stage(JobStage::Complete);
        assert_eq!(job.stage, JobStage::Complete);
    }

    #[test]
    fn test_job_set_status() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::Unknown, &temp_dir);

        job.set_status(JobStatus::Running);
        assert_eq!(job.status, JobStatus::Running);

        job.set_status(JobStatus::Success);
        assert_eq!(job.status, JobStatus::Success);
    }

    #[test]
    fn test_job_fail() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::Unknown, &temp_dir);

        job.fail("Encoding failed: av1an exited with code 1");

        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error_reason,
            Some("Encoding failed: av1an exited with code 1".to_string())
        );
    }

    #[test]
    fn test_job_skip() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::Unknown, &temp_dir);

        job.skip("Size gate rejected: output larger than original");

        assert_eq!(job.status, JobStatus::Skipped);
        assert_eq!(
            job.error_reason,
            Some("Size gate rejected: output larger than original".to_string())
        );
    }

    #[test]
    fn test_job_is_terminal() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::Unknown, &temp_dir);

        // Pending is not terminal
        assert!(!job.is_terminal());

        // Running is not terminal
        job.set_status(JobStatus::Running);
        assert!(!job.is_terminal());

        // Success is terminal
        job.set_status(JobStatus::Success);
        assert!(job.is_terminal());

        // Failed is terminal
        job.set_status(JobStatus::Failed);
        assert!(job.is_terminal());

        // Skipped is terminal
        job.set_status(JobStatus::Skipped);
        assert!(job.is_terminal());
    }

    #[test]
    fn test_job_is_active() {
        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job = create_job(&candidate, probe, SourceType::Unknown, &temp_dir);

        // Pending is active
        assert!(job.is_active());

        // Running is active
        job.set_status(JobStatus::Running);
        assert!(job.is_active());

        // Success is not active
        job.set_status(JobStatus::Success);
        assert!(!job.is_active());

        // Failed is not active
        job.set_status(JobStatus::Failed);
        assert!(!job.is_active());

        // Skipped is not active
        job.set_status(JobStatus::Skipped);
        assert!(!job.is_active());
    }

    #[test]
    fn test_save_and_load_job() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path();

        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let output_dir = PathBuf::from("/tmp/av1-daemon");

        let job = create_job(&candidate, probe, SourceType::DiscLike, &output_dir);
        let job_id = job.id.clone();

        // Save job
        save_job(&job, state_dir).expect("Should save job");

        // Verify file exists
        let job_file = state_dir.join(format!("{}.json", job_id));
        assert!(job_file.exists());

        // Load jobs
        let loaded_jobs = load_jobs(state_dir).expect("Should load jobs");

        assert_eq!(loaded_jobs.len(), 1);
        assert_eq!(loaded_jobs[0].id, job_id);
        assert_eq!(loaded_jobs[0].input_path, job.input_path);
        assert_eq!(loaded_jobs[0].stage, job.stage);
        assert_eq!(loaded_jobs[0].status, job.status);
        assert_eq!(loaded_jobs[0].source_type, job.source_type);
    }

    #[test]
    fn test_load_jobs_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path();

        let jobs = load_jobs(state_dir).expect("Should load from empty dir");
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_load_jobs_nonexistent_dir() {
        let jobs = load_jobs(Path::new("/nonexistent/path/that/does/not/exist"))
            .expect("Should return empty for nonexistent dir");
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_job_exists_for_path() {
        let candidate1 = make_scan_candidate("/media/movies/film1.mkv");
        let candidate2 = make_scan_candidate("/media/movies/film2.mkv");
        let probe = make_probe_result();
        let temp_dir = PathBuf::from("/tmp/av1-daemon");

        let mut job1 = create_job(&candidate1, probe.clone(), SourceType::Unknown, &temp_dir);
        let mut job2 = create_job(&candidate2, probe.clone(), SourceType::Unknown, &temp_dir);

        // Job1 is pending (active)
        // Job2 is completed (not active)
        job2.set_status(JobStatus::Success);

        let jobs = vec![job1.clone(), job2.clone()];

        // Should find active job for film1
        assert!(job_exists_for_path(&jobs, Path::new("/media/movies/film1.mkv")));

        // Should NOT find job for film2 (completed, not active)
        assert!(!job_exists_for_path(&jobs, Path::new("/media/movies/film2.mkv")));

        // Should NOT find job for unknown path
        assert!(!job_exists_for_path(&jobs, Path::new("/media/movies/film3.mkv")));

        // If job1 becomes running, should still find it
        job1.set_status(JobStatus::Running);
        let jobs = vec![job1, job2];
        assert!(job_exists_for_path(&jobs, Path::new("/media/movies/film1.mkv")));
    }

    #[test]
    fn test_save_job_creates_directory() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("nested/state/dir");

        let candidate = make_scan_candidate("/media/movies/film.mkv");
        let probe = make_probe_result();
        let output_dir = PathBuf::from("/tmp/av1-daemon");

        let job = create_job(&candidate, probe, SourceType::Unknown, &output_dir);

        // Save should create the directory
        save_job(&job, &state_dir).expect("Should save job and create dir");

        assert!(state_dir.exists());
        assert!(state_dir.join(format!("{}.json", job.id)).exists());
    }
}
