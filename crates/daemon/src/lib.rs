//! AV1 Super Daemon
//!
//! Background service that manages the encoding pipeline, job queue, and metrics collection.

pub mod concurrency;
pub mod daemon;
pub mod encode;
pub mod job_executor;
pub mod metrics;
pub mod metrics_server;
pub mod startup;

pub use av1_super_daemon_config as config;
pub use av1_super_daemon_config::Config;
pub use concurrency::{derive_plan, ConcurrencyPlan};
pub use daemon::{Daemon, DaemonError};
pub use encode::{build_av1an_command, run_av1an, Av1anEncodeParams, EncodeError};
pub use job_executor::{Job, JobError, JobExecutor, JobState};
pub use metrics::{
    collect_system_metrics, new_shared_metrics, JobMetrics, MetricsSnapshot, SharedMetrics,
    SystemMetrics,
};
pub use metrics_server::{create_metrics_router, run_metrics_server, ServerError};
pub use startup::{
    assert_software_only, check_args_for_hardware_flags, check_av1an_available,
    check_ffmpeg_version_8_or_newer, detect_hardware_flag, parse_ffmpeg_version,
    run_startup_checks, StartupError,
};
