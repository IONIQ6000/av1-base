//! AV1 Super Daemon
//!
//! Background service that manages the encoding pipeline, job queue, and metrics collection.

pub mod classify;
pub mod concurrency;
pub mod daemon;
pub mod encode;
pub mod gates;
pub mod job_executor;
pub mod jobs;
pub mod metrics;
pub mod metrics_server;
pub mod replace;
pub mod scan;
pub mod size_gate;
pub mod skip_marker;
pub mod stability;
pub mod startup;

pub use av1_super_daemon_config as config;
pub use av1_super_daemon_config::Config;
pub use concurrency::{derive_plan, ConcurrencyPlan};
pub use daemon::{Daemon, DaemonError};
pub use encode::{build_av1an_command, run_av1an, Av1anEncodeParams, EncodeError};
pub use job_executor::{Job, JobError, JobExecutor, JobExecutorConfig, JobState};
pub use metrics::{
    collect_system_metrics, new_shared_metrics, JobMetrics, MetricsSnapshot, SharedMetrics,
    SystemMetrics,
};
pub use metrics_server::{create_metrics_router, run_metrics_server, ServerError};
pub use scan::{
    has_skip_marker, is_video_file, scan_libraries, skip_marker_path, ScanCandidate,
    VIDEO_EXTENSIONS,
};
pub use stability::{check_stability, compare_sizes, StabilityResult};
pub use startup::{
    assert_software_only, check_args_for_hardware_flags, check_av1an_available,
    check_ffmpeg_version_8_or_newer, detect_hardware_flag, parse_ffmpeg_version,
    run_startup_checks, StartupError,
};
pub use gates::{
    check_gates, parse_ffprobe_output, probe_file, AudioStream, FormatInfo, GateResult,
    GatesConfig, ProbeError, ProbeResult, VideoStream,
};
pub use classify::{classify_source, SourceType};
pub use jobs::{
    create_job, job_exists_for_path, load_jobs, save_job, Job as ManagedJob, JobStage, JobStatus,
};
pub use size_gate::{check_size_gate, SizeGateResult};
pub use skip_marker::{why_sidecar_path, write_skip_marker, write_why_sidecar};
pub use replace::{atomic_replace, backup_path, ReplaceError};
