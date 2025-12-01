# Requirements Document

## Introduction

The AV1 Super Daemon is a Rust-based system that automates media encoding workflows using AV1 compression with film-grain tuning. The system consists of a daemon that scans library directories, gates candidates, encodes media files using Av1an (built from source) with SVT-AV1 encoder, validates output, and atomically replaces originals. It enforces software-only encoding (no GPU/hardware acceleration), requires FFmpeg 8+, persists job state as JSON, uses skip markers to prevent reprocessing, and provides a rich TUI dashboard built with Ratatui for real-time monitoring of encoding jobs, system metrics, and throughput.

## Glossary

- **Av1an**: A chunked video encoding tool that parallelizes AV1 encoding across multiple workers
- **SVT-AV1**: Scalable Video Technology for AV1, a software-based AV1 encoder
- **CRF**: Constant Rate Factor, a quality-based encoding parameter (lower = higher quality)
- **Film-grain tuning**: Encoder settings optimized for preserving film grain texture
- **TUI**: Terminal User Interface
- **Ratatui**: A Rust library for building terminal user interfaces
- **Daemon**: A background process that runs continuously to process encoding jobs
- **Gate**: A validation checkpoint that determines if a file should proceed through the pipeline
- **VMAF**: Video Multimethod Assessment Fusion, a perceptual video quality metric
- **PSNR**: Peak Signal-to-Noise Ratio, an objective video quality metric
- **SSIM**: Structural Similarity Index, an image/video quality metric
- **Concurrency Plan**: Configuration determining worker count and concurrent job limits based on CPU cores
- **Library Root**: A configured directory path that the daemon recursively scans for video files
- **Skip Marker**: A `.av1skip` sidecar file indicating a video should not be processed
- **Why Sidecar**: A `.why.txt` file explaining why a file was skipped
- **Stability Check**: Verification that a file's size remains unchanged over a time window before processing
- **Size Gate**: Post-encode validation ensuring output is smaller than original by a configured ratio
- **Job State Directory**: Directory where job JSON files are persisted for recovery and TUI consumption
- **Atomic Replacement**: Safe file swap with backup creation before overwriting original
- **Source Classification**: Categorization of video as WebLike or DiscLike based on path/codec/bitrate heuristics

## Requirements

### Requirement 1

**User Story:** As a system administrator, I want the daemon to automatically derive optimal encoding concurrency settings from CPU core count, so that I can maximize hardware utilization without manual tuning.

#### Acceptance Criteria

1. WHEN the daemon starts with no explicit worker configuration THEN the Daemon SHALL derive `av1an_workers=8` for hosts with 32 or more logical cores
2. WHEN the daemon starts with no explicit worker configuration on hosts with fewer than 32 cores THEN the Daemon SHALL derive `av1an_workers=4`
3. WHEN the daemon starts with no explicit concurrent jobs configuration on hosts with 24 or more cores THEN the Daemon SHALL derive `max_concurrent_jobs=1`
4. WHEN the daemon starts with explicit `workers_per_job` or `max_concurrent_jobs` values in configuration THEN the Daemon SHALL use those explicit values instead of derived defaults
5. WHEN the daemon computes target thread utilization THEN the Daemon SHALL clamp `target_cpu_utilization` between 0.5 and 1.0

### Requirement 2

**User Story:** As a media engineer, I want the daemon to encode video using specific film-grain-tuned AV1 settings, so that I get consistent high-quality output optimized for film content.

#### Acceptance Criteria

1. WHEN the Daemon encodes a video file THEN the Daemon SHALL invoke Av1an with encoder set to `svt-av1`
2. WHEN the Daemon encodes a video file THEN the Daemon SHALL use pixel format `yuv420p10le`
3. WHEN the Daemon encodes a video file THEN the Daemon SHALL use CRF value `8`
4. WHEN the Daemon encodes a video file THEN the Daemon SHALL use preset `3`
5. WHEN the Daemon encodes a video file THEN the Daemon SHALL pass SVT parameters `tune=grain:film-grain=20:enable-qm=1:qm-min=1:qm-max=15:keyint=240:lookahead=40`
6. WHEN the Daemon encodes a video file THEN the Daemon SHALL set `--target-quality 1`
7. WHEN the Daemon encodes a video file THEN the Daemon SHALL copy audio streams using `--audio-copy`

### Requirement 3

**User Story:** As a system administrator, I want the daemon to enforce software-only encoding, so that I can ensure consistent behavior across different hardware configurations without GPU dependencies.

#### Acceptance Criteria

1. WHEN `disallow_hardware_encoding` is enabled and configuration contains hardware encoder flags THEN the Daemon SHALL reject the configuration and abort startup
2. WHEN the Daemon checks for forbidden hardware flags THEN the Daemon SHALL detect flags containing `nvenc`, `qsv`, `vaapi`, `cuda`, `amf`, `vce`, or `qsvenc`
3. WHEN the Daemon invokes FFmpeg for probing or remuxing THEN the Daemon SHALL omit `-hwaccel`, `-hwaccel_device`, and hardware encoder arguments

### Requirement 4

**User Story:** As a system administrator, I want the daemon to verify that required external tools are available and meet version requirements at startup, so that encoding jobs do not fail due to missing dependencies.

#### Acceptance Criteria

1. WHEN the daemon starts THEN the Daemon SHALL verify that `av1an --version` executes successfully
2. WHEN `av1an --version` fails THEN the Daemon SHALL abort startup with an error message indicating Av1an is unavailable
3. WHEN the daemon starts THEN the Daemon SHALL verify that FFmpeg version is 8.0 or newer
4. WHEN FFmpeg version is below 8.0 THEN the Daemon SHALL abort startup with an error message indicating the required version
5. WHEN parsing FFmpeg version THEN the Daemon SHALL handle version strings prefixed with `n` (e.g., `n8.0-...`)

### Requirement 5

**User Story:** As a media engineer, I want the daemon to execute encoding jobs through a managed pipeline, so that files are properly validated and replaced after successful encoding.

#### Acceptance Criteria

1. WHEN a job reaches the encode stage THEN the Daemon SHALL create a temporary chunks directory for Av1an processing
2. WHEN Av1an encoding completes successfully THEN the Daemon SHALL proceed to validation and size gate stages
3. WHEN Av1an encoding fails THEN the Daemon SHALL mark the job as failed and halt processing for that job
4. WHEN validation passes THEN the Daemon SHALL replace the original file with the encoded output
5. WHEN the Daemon executes encoding jobs THEN the Daemon SHALL respect the `max_concurrent_jobs` limit from the concurrency plan

### Requirement 6

**User Story:** As a system operator, I want to monitor encoding progress and system health through a TUI dashboard, so that I can observe job status and resource utilization in real-time.

#### Acceptance Criteria

1. WHEN the TUI dashboard starts THEN the Dashboard SHALL connect to the daemon metrics endpoint at `http://127.0.0.1:7878/metrics`
2. WHEN the TUI dashboard receives metrics THEN the Dashboard SHALL display a queue table with columns for ID, Stage, Progress %, FPS, Bitrate, CRF, Workers, and ETA
3. WHEN the TUI dashboard receives metrics THEN the Dashboard SHALL display system gauges for CPU usage percentage and memory usage percentage
4. WHEN the TUI dashboard receives metrics THEN the Dashboard SHALL display load averages (1, 5, and 15 minute)
5. WHEN the TUI dashboard receives metrics THEN the Dashboard SHALL display a throughput chart showing MB encoded over time
6. WHEN the TUI dashboard receives metrics THEN the Dashboard SHALL display an event log with recent job events
7. WHEN the TUI dashboard polls for updates THEN the Dashboard SHALL refresh approximately every 500 milliseconds

### Requirement 7

**User Story:** As a system operator, I want the daemon to expose encoding and system metrics via HTTP, so that the TUI dashboard and other monitoring tools can consume real-time data.

#### Acceptance Criteria

1. WHEN the daemon starts THEN the Daemon SHALL start an HTTP server on `127.0.0.1:7878`
2. WHEN a client requests `/metrics` THEN the Daemon SHALL respond with a JSON-serialized `MetricsSnapshot`
3. WHEN the Daemon updates metrics THEN the Daemon SHALL include per-job metrics: id, input_path, stage, progress, fps, bitrate_kbps, crf, encoder, workers, est_remaining_secs, frames_encoded, total_frames, size_in_bytes_before, size_in_bytes_after, and optional quality metrics (vmaf, psnr, ssim)
4. WHEN the Daemon updates metrics THEN the Daemon SHALL include system metrics: cpu_usage_percent, mem_usage_percent, load_avg_1, load_avg_5, load_avg_15
5. WHEN the Daemon updates metrics THEN the Daemon SHALL include aggregate metrics: queue_len, running_jobs, completed_jobs, failed_jobs, total_bytes_encoded

### Requirement 8

**User Story:** As a developer, I want configuration to be loadable from a TOML file with environment variable overrides, so that I can deploy the daemon flexibly across different environments.

#### Acceptance Criteria

1. WHEN the daemon loads configuration THEN the Daemon SHALL parse `config.toml` for cpu, av1an, and encoder_safety sections
2. WHEN environment variable `CPU_LOGICAL_CORES` is set THEN the Daemon SHALL override the configured `logical_cores` value
3. WHEN environment variable `CPU_TARGET_UTILIZATION` is set THEN the Daemon SHALL override the configured `target_cpu_utilization` value
4. WHEN environment variable `AV1AN_WORKERS_PER_JOB` is set THEN the Daemon SHALL override the configured `workers_per_job` value
5. WHEN environment variable `AV1AN_MAX_CONCURRENT_JOBS` is set THEN the Daemon SHALL override the configured `max_concurrent_jobs` value
6. WHEN environment variable `ENCODER_DISALLOW_HARDWARE_ENCODING` is set THEN the Daemon SHALL override the configured `disallow_hardware_encoding` value

### Requirement 9

**User Story:** As a system administrator, I want an installation script for FFmpeg 8+ on Debian hosts, so that I can quickly provision encoding servers with the required dependencies.

#### Acceptance Criteria

1. WHEN the install script runs without root privileges THEN the Script SHALL exit with an error message
2. WHEN the install script runs THEN the Script SHALL remove any existing distro-installed FFmpeg
3. WHEN the install script runs THEN the Script SHALL download FFmpeg from the URL specified in `FFMPEG_ARCHIVE_URL` environment variable
4. WHEN `FFMPEG_ARCHIVE_URL` is not set THEN the Script SHALL exit with an error message
5. WHEN the install script completes THEN the Script SHALL install ffmpeg and ffprobe to `/usr/local/bin`
6. WHEN the install script completes THEN the Script SHALL display the installed FFmpeg version

### Requirement 10

**User Story:** As a developer, I want the Av1an command builder to produce correct CLI arguments, so that encoding jobs use the exact specified parameters.

#### Acceptance Criteria

1. WHEN building an Av1an command THEN the Command Builder SHALL include `-i` with the input path
2. WHEN building an Av1an command THEN the Command Builder SHALL include `-o` with the output path
3. WHEN building an Av1an command THEN the Command Builder SHALL include `--encoder svt-av1`
4. WHEN building an Av1an command THEN the Command Builder SHALL include `--pix-format yuv420p10le`
5. WHEN building an Av1an command THEN the Command Builder SHALL include `--crf 8`
6. WHEN building an Av1an command THEN the Command Builder SHALL include `--preset 3`
7. WHEN building an Av1an command THEN the Command Builder SHALL include `--svt-params` with the film-grain tuning string
8. WHEN building an Av1an command THEN the Command Builder SHALL include `--target-quality 1`
9. WHEN building an Av1an command THEN the Command Builder SHALL include `--audio-copy`
10. WHEN building an Av1an command THEN the Command Builder SHALL include `--workers` with the value from the concurrency plan
11. WHEN building an Av1an command THEN the Command Builder SHALL include `--temp` with the temporary chunks directory path

### Requirement 11

**User Story:** As a system administrator, I want the daemon to scan configured library directories for video files, so that new media is automatically discovered and queued for encoding.

#### Acceptance Criteria

1. WHEN the daemon runs a scan cycle THEN the Scanner SHALL recursively walk each configured `library_root` directory
2. WHEN the Scanner encounters a hidden directory (name starting with `.`) THEN the Scanner SHALL skip that directory and its contents
3. WHEN the Scanner encounters a file THEN the Scanner SHALL consider only video extensions: `.mkv`, `.mp4`, `.avi`, `.mov`, `.m4v`, `.ts`, `.m2ts`
4. WHEN the Scanner encounters a file with a corresponding `.av1skip` marker THEN the Scanner SHALL skip that file
5. WHEN the Scanner discovers a candidate file THEN the Scanner SHALL capture the file size and modified time for stability checking

### Requirement 12

**User Story:** As a media engineer, I want the daemon to verify file stability before processing, so that files still being written or transferred are not corrupted by premature encoding.

#### Acceptance Criteria

1. WHEN a candidate file is discovered THEN the Stability Checker SHALL wait a configurable duration (default 10 seconds)
2. WHEN the stability wait completes THEN the Stability Checker SHALL compare the current file size to the initially captured size
3. WHEN the file size has changed during the stability window THEN the Stability Checker SHALL mark the file as unstable and retry on the next scan cycle
4. WHEN the file size remains unchanged THEN the Stability Checker SHALL mark the file as stable and allow processing to continue

### Requirement 13

**User Story:** As a media engineer, I want the daemon to gate files based on probe results, so that unsuitable files are skipped with clear explanations.

#### Acceptance Criteria

1. WHEN a stable file is ready for gating THEN the Gate Checker SHALL run `ffprobe` to collect stream and format metadata
2. WHEN `ffprobe` fails on a file THEN the Gate Checker SHALL create a `.av1skip` marker and optionally a `.why.txt` sidecar explaining the probe failure
3. WHEN a file has no video streams THEN the Gate Checker SHALL skip the file and create skip markers with reason "no video streams"
4. WHEN a file is smaller than the configured `min_bytes` threshold THEN the Gate Checker SHALL skip the file with reason "below minimum size"
5. WHEN the first video stream is already AV1 codec THEN the Gate Checker SHALL skip the file with reason "already AV1"
6. WHEN a file passes all gates THEN the Gate Checker SHALL allow the file to proceed to job creation

### Requirement 14

**User Story:** As a system operator, I want job state persisted as JSON files, so that the TUI can display live status and jobs can survive daemon restarts.

#### Acceptance Criteria

1. WHEN a job is created THEN the Job Manager SHALL persist a JSON file in the configured `job_state_dir` with job metadata
2. WHEN a job state changes (stage, progress, status) THEN the Job Manager SHALL update the corresponding JSON file
3. WHEN the daemon starts THEN the Job Manager SHALL load existing job JSON files to avoid duplicate work on in-flight items
4. WHEN a job completes or fails THEN the Job Manager SHALL update the JSON file with final status and reason

### Requirement 15

**User Story:** As a media engineer, I want the daemon to classify source files, so that encoding parameters can be adjusted for web-sourced vs disc-sourced content.

#### Acceptance Criteria

1. WHEN a file passes gates THEN the Classifier SHALL analyze path keywords, bitrate vs resolution ratio, and codec to determine source type
2. WHEN path contains web-related keywords or bitrate is low relative to resolution THEN the Classifier SHALL label the source as WebLike
3. WHEN path contains disc-related keywords or bitrate is high relative to resolution THEN the Classifier SHALL label the source as DiscLike
4. WHEN classification cannot be determined THEN the Classifier SHALL label the source as Unknown
5. WHEN a source is classified THEN the Job SHALL store the `is_web_like` flag for later use

### Requirement 16

**User Story:** As a media engineer, I want post-encode size validation, so that encodes that grow larger than the original are rejected.

#### Acceptance Criteria

1. WHEN encoding completes successfully THEN the Size Gate SHALL compare output file size to original file size
2. WHEN output size is greater than or equal to `original_size * max_size_ratio` THEN the Size Gate SHALL reject the encode
3. WHEN the Size Gate rejects an encode THEN the Daemon SHALL delete the temp output, create `.av1skip` marker, and optionally create `.why.txt` with reason
4. WHEN the Size Gate accepts an encode THEN the Daemon SHALL proceed to file replacement

### Requirement 17

**User Story:** As a system administrator, I want atomic file replacement with backup, so that original files are protected during the swap process.

#### Acceptance Criteria

1. WHEN replacement begins THEN the Replacer SHALL create a backup of the original file as `<name>.orig.<timestamp>`
2. WHEN backup creation fails THEN the Replacer SHALL abort replacement and preserve both original and encoded files
3. WHEN backup succeeds THEN the Replacer SHALL copy the encoded file to the original location
4. WHEN copy succeeds and `keep_original` is false THEN the Replacer SHALL delete the backup file
5. WHEN copy succeeds and `keep_original` is true THEN the Replacer SHALL preserve the backup file
6. WHEN any replacement step fails THEN the Replacer SHALL preserve temp files for manual inspection and mark job as failed

### Requirement 18

**User Story:** As a developer, I want skip markers and why sidecars, so that skipped files are documented and not reprocessed.

#### Acceptance Criteria

1. WHEN a file is skipped for any reason THEN the Skip Marker Writer SHALL create a `.av1skip` file adjacent to the original
2. WHEN `write_why_sidecars` is enabled THEN the Skip Marker Writer SHALL create a `.why.txt` file with the skip reason
3. WHEN a `.av1skip` marker exists for a file THEN the Scanner SHALL not queue that file for processing
4. WHEN checking for skip markers THEN the Daemon SHALL look for `<filename>.av1skip` in the same directory
