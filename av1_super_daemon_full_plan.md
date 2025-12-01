# AV1 Super Daemon Plan (Updated Av1an Settings)
## Debian host • Av1an from source • Film-grain tuned AV1 • FFmpeg 8+ • Software-only • Full TUI metrics

This document is a **single, self-contained build plan** for an AI IDE.

It describes how to build a system that:

- Runs a Rust **daemon** that scans / gates / encodes / validates / replaces media.
- Builds and uses **Av1an** from source (`https://github.com/rust-av/Av1an`) alongside your daemon.
- Uses Av1an’s **parallel workers** to fully exploit a **32-core CPU**.
- Uses a **specific, film-grain-tuned AV1 encoding configuration**:

  ```bash
  av1an     -i "input.mkv"     -o "output.mkv"     --encoder svt-av1     --pix-format yuv420p10le     --crf 8     --preset 3     --svt-params "tune=grain:film-grain=20:enable-qm=1:qm-min=1:qm-max=15:keyint=240:lookahead=40"     --target-quality 1     --audio-copy     --workers 8     --temp "temp_chunks"
  ```

- Enforces **software-only encoding** (no GPU / hardware encoders).
- Installs and **mandates FFmpeg 8+** on a Debian host.
- Provides a rich **TUI dashboard** (Ratatui) with detailed metrics:
  - Per-job encoding metrics (fps, bitrate, frames, size, quality, etc.).
  - Queue metrics.
  - System metrics (CPU, memory, load).
  - Throughput over time and event logs.

Assumptions:

- You already have a Rust monorepo with at least:
  - `crates/cli-daemon` – CLI entrypoint.
  - `crates/daemon` – main logic (startup, job loop, gates, encode, validate, replace).
- Encoding currently uses direct FFmpeg calls that you want to replace with Av1an while preserving the rest of the pipeline.

---

## 1. Vendoring Av1an from source

### 1.1. Add Av1an as a submodule

(Commands for reference; the IDE can emulate the net effect.)

```bash
git submodule add https://github.com/rust-av/Av1an.git third_party/av1an
```

Expected layout:

- `third_party/av1an/Cargo.toml`
- `third_party/av1an/src/main.rs`
- …

### 1.2. Add Av1an to the Cargo workspace

In your top-level `Cargo.toml`:

```toml
[workspace]
members = [
  "crates/cli-daemon",
  "crates/daemon",
  # ... other members ...
  "third_party/av1an",
]
```

Now `cargo build --release` will also build `target/release/av1an` on the host.

---

## 2. Configuration: CPU, Av1an, software safety

### 2.1. CPU & Av1an config structs

In `crates/config/src/lib.rs` (or equivalent) add:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct CpuConfig {
    /// Logical cores on the host. If None, auto-detect via num_cpus.
    pub logical_cores: Option<u32>,

    /// Target fraction of CPU utilization for AV1 encoding (0.5–1.0).
    #[serde(default = "default_target_cpu_utilization")]
    pub target_cpu_utilization: f32,
}

fn default_target_cpu_utilization() -> f32 {
    0.85
}

#[derive(Debug, Clone, Deserialize)]
pub struct Av1anConfig {
    /// Override workers per job. If 0, auto-derive from logical cores.
    #[serde(default)]
    pub workers_per_job: u32,

    /// Max number of concurrent encoding jobs.
    /// If 0, auto-derive based on core count.
    #[serde(default)]
    pub max_concurrent_jobs: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EncoderSafetyConfig {
    /// If true, reject any config that attempts to use hardware encoders/accel.
    #[serde(default = "default_disallow_hw")]
    pub disallow_hardware_encoding: bool,
}

fn default_disallow_hw() -> bool { true }

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    // existing fields…
    #[serde(default)]
    pub cpu: CpuConfig,

    #[serde(default)]
    pub av1an: Av1anConfig,

    #[serde(default)]
    pub encoder_safety: EncoderSafetyConfig,
}
```

We’re no longer exposing encoder choice or CRF/preset in config; the encoding settings are **fixed** to the command shown above.

### 2.2. `config.toml` example

```toml
[cpu]
# Set explicitly for a known 32-core host, or leave unset to auto-detect.
logical_cores = 32
target_cpu_utilization = 0.85

[av1an]
# Leave 0 to derive defaults. For a 32-core CPU this will resolve to workers = 8, jobs = 1.
workers_per_job = 0
max_concurrent_jobs = 1

[encoder_safety]
# Enforce software-only encoding.
disallow_hardware_encoding = true
```

### 2.3. Environment overrides (optional)

Allow env vars like:

- `CPU_LOGICAL_CORES`
- `CPU_TARGET_UTILIZATION`
- `AV1AN_WORKERS_PER_JOB`
- `AV1AN_MAX_CONCURRENT_JOBS`
- `ENCODER_DISALLOW_HARDWARE_ENCODING`

to override these values at runtime.

---

## 3. Concurrency planning for a 32-core CPU

Create `crates/daemon/src/concurrency.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ConcurrencyPlan {
    pub total_cores: u32,
    pub target_threads: u32,
    pub av1an_workers: u32,
    pub max_concurrent_jobs: u32,
}
```

Add:

```rust
use config::Config;

pub fn derive_plan(cfg: &Config) -> ConcurrencyPlan {
    let detected_cores = num_cpus::get() as u32;
    let total_cores = cfg.cpu.logical_cores.unwrap_or(detected_cores);

    let mut target_util = cfg.cpu.target_cpu_utilization;
    if target_util < 0.5 {
        target_util = 0.5;
    } else if target_util > 1.0 {
        target_util = 1.0;
    }

    let target_threads = ((total_cores as f32) * target_util).round() as u32;

    let mut workers = cfg.av1an.workers_per_job;
    if workers == 0 {
        // For this project we want workers=8 on a 32-core CPU by default.
        workers = if total_cores >= 32 { 8 } else { 4 };
    }

    let mut max_jobs = cfg.av1an.max_concurrent_jobs;
    if max_jobs == 0 {
        // Heavy encodes: default to 1 job for 24+ cores.
        max_jobs = if total_cores >= 24 { 1 } else { 2 };
    }

    ConcurrencyPlan {
        total_cores,
        target_threads,
        av1an_workers: workers,
        max_concurrent_jobs: max_jobs,
    }
}
```

In `startup.rs`:

```rust
let plan = concurrency::derive_plan(&config);
log::info!(
    "Av1an concurrency: {} job(s) * {} workers/job on {} cores (target-util ~{:.0}%)",
    plan.max_concurrent_jobs,
    plan.av1an_workers,
    plan.total_cores,
    config.cpu.target_cpu_utilization * 100.0,
);
// store `plan` in shared state
```

On a 32-core machine, default outcome:

- `av1an_workers = 8`
- `max_concurrent_jobs = 1`

matching the CLI example.

---

## 4. Software-only encoding guarantees

We must ensure *only* CPU encoders are used and no GPU acceleration is accidentally enabled.

### 4.1. FFmpeg usage: no hardware acceleration flags

Anywhere you call FFmpeg (including validation, probing, or occasional remuxes):

- Do **not** pass `-hwaccel`, `-hwaccel_device`, or hardware encoders like `h264_nvenc`, `av1_qsv`, etc.
- For this plan, FFmpeg is used only for:
  - Probing / validation (via `ffprobe` or `ffmpeg -i`).
  - Optional remux or stream-mapping operations (if you decide to keep them).

Encoding itself is handled by Av1an with **SVT-AV1**.

### 4.2. Safety preflight

If your config allows free-form FFmpeg/encoder args, add a guard in `startup.rs`:

```rust
fn assert_software_only(cfg: &Config) -> Result<(), String> {
    if !cfg.encoder_safety.disallow_hardware_encoding {
        return Ok(());
    }

    let forbidden = ["nvenc", "qsv", "vaapi", "cuda", "amf", "vce", "qsvenc"];

    // Collect any user-supplied arg strings from config here.
    let user_args: Vec<String> = vec![
        // e.g. cfg.encode.extra_ffmpeg_args.clone()
    ];

    for arg in user_args {
        let lower = arg.to_lowercase();
        if let Some(&bad) = forbidden.iter().find(|f| lower.contains(**f)) {
            return Err(format!(
                "Hardware encoding flag '{}' found in '{}', but hardware encoding is disabled",
                bad, arg
            ));
        }
    }

    Ok(())
}
```

Call `assert_software_only(&config)` during startup and abort if it fails.

---

## 5. Av1an encoding module (using the requested settings)

We now wrap the exact Av1an CLI configuration you provided.

Create `crates/daemon/src/encode/av1an.rs`:

```rust
use std::path::PathBuf;
use std::process::Command;
use std::io;

use crate::concurrency::ConcurrencyPlan;

pub struct Av1anEncodeParams {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub temp_chunks_dir: PathBuf,
    pub concurrency: ConcurrencyPlan,
}

/// Build an Av1an command using the requested film-grain-tuned configuration:
///
/// av1an \
///   -i "input.mkv" \
///   -o "output.mkv" \
///   --encoder svt-av1 \
///   --pix-format yuv420p10le \
///   --crf 8 \
///   --preset 3 \
///   --svt-params "tune=grain:film-grain=20:enable-qm=1:qm-min=1:qm-max=15:keyint=240:lookahead=40" \
///   --target-quality 1 \
///   --audio-copy \
///   --workers 8 \
///   --temp "temp_chunks"
pub fn build_av1an_command(params: &Av1anEncodeParams) -> Command {
    let mut cmd = Command::new("av1an");

    cmd.arg("-i").arg(&params.input_path)
        .arg("-o").arg(&params.output_path)
        .arg("--encoder").arg("svt-av1")
        .arg("--pix-format").arg("yuv420p10le")
        .arg("--crf").arg("8")
        .arg("--preset").arg("3")
        .arg("--svt-params").arg(
            "tune=grain:film-grain=20:enable-qm=1:qm-min=1:qm-max=15:keyint=240:lookahead=40",
        )
        .arg("--target-quality").arg("1")
        .arg("--audio-copy")
        // Workers derived from concurrency plan; default = 8 on 32-core CPU.
        .arg("--workers")
        .arg(params.concurrency.av1an_workers.to_string())
        .arg("--temp")
        .arg(&params.temp_chunks_dir);

    cmd
}

pub fn run_av1an(params: &Av1anEncodeParams) -> io::Result<()> {
    let mut cmd = build_av1an_command(params);
    let status = cmd.status()?;

    if !status.success() {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("av1an failed with status: {status}"),
        ))
    } else {
        Ok(())
    }
}
```

Notes:

- We keep `workers` dynamic via `ConcurrencyPlan`, but on a 32-core CPU it will default to `8`, matching your CLI.
- Av1an handles:
  - Video encoding (SVT-AV1).
  - Audio copying (`--audio-copy`).
  - Chunk temp storage in `temp_chunks_dir`.

The output file from Av1an is directly the **final encoded file** (no separate remux stage required unless you have extra mapping rules you want to preserve via FFmpeg).

---

## 6. Encoding pipeline changes

Your original pipeline did roughly:

1. FFmpeg encodes video + maps streams.
2. Validation & size gate.
3. Replacement.

We replace the **encode** step with Av1an using the above settings, while keeping everything else intact.

In `crates/daemon/src/encode/mod.rs` (or equivalent):

1. When a job reaches the encode stage:

   ```rust
   use crate::encode::av1an::{Av1anEncodeParams, run_av1an};

   let temp_dir = temp_output_root.join(format!("{}_chunks", job.id));
   let final_output_path = temp_output_root.join(format!("{}.mkv", job.id));

   let params = Av1anEncodeParams {
       input_path: original_input_path.clone(),
       output_path: final_output_path.clone(),
       temp_chunks_dir: temp_dir.clone(),
       concurrency: concurrency_plan.clone(),
   };

   run_av1an(&params)?;
   ```

2. On success, proceed to your existing **validation + size gate + replacement** steps, treating `final_output_path` as the encoded candidate.
3. On failure, mark the job as failed and stop.

FFmpeg 8 remains available for:

- Probing (duration, stream codecs, etc.).
- Optional remux or stream-mapping operations if you decide to keep a separate mapping stage.

---

## 7. Wiring concurrency into the job executor

In `crates/daemon/src/daemon_loop.rs` (or wherever you create the worker pool):

```rust
let permits = concurrency_plan.max_concurrent_jobs;
let (executor, handle) = JobExecutor::new(permits, /* ... */);
```

Ensure you pass `concurrency_plan` into whatever context is used when constructing `Av1anEncodeParams` so `--workers` is derived correctly.

Default on 32-core:

- `max_concurrent_jobs = 1`
- `av1an_workers = 8`

So you’ll have **one Av1an encode running with eight workers**, matching your CLI and giving very heavy CPU use.

---

## 8. Startup checks: Av1an & FFmpeg 8+

In `crates/daemon/src/startup.rs`:

```rust
use std::process::Command;
use std::io;

pub fn check_av1an_available() -> io::Result<()> {
    let status = Command::new("av1an")
        .arg("--version")
        .status()?;

    if !status.success() {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "av1an --version failed; is Av1an built and in PATH?",
        ))
    } else {
        Ok(())
    }
}

pub fn check_ffmpeg_version_8_or_newer() -> io::Result<()> {
    let output = Command::new("ffmpeg")
        .arg("-version")
        .output()?;

    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "ffmpeg -version failed; is FFmpeg installed?",
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or("");

    let major_ok = first_line
        .split_whitespace()
        .nth(2)              // e.g. "8.0.1" or "n8.0-..."
        .and_then(|v| {
            let trimmed = v.trim_start_matches('n');
            trimmed.split('.').next()
        })
        .and_then(|m| m.parse::<u32>().ok())
        .map(|major| major >= 8)
        .unwrap_or(false);

    if !major_ok {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("FFmpeg 8.x required, got: {first_line}"),
        ));
    }

    Ok(())
}
```

Call, in order:

1. `assert_software_only(&config)`
2. `check_av1an_available()`
3. `check_ffmpeg_version_8_or_newer()`
4. `derive_plan(&config)`

Abort startup if any step fails.

---

## 9. FFmpeg 8 installer script (Debian host)

Create `scripts/install_ffmpeg8.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
  echo "This script must be run as root (sudo)." >&2
  exit 1
fi

echo "[ffmpeg8] Removing any distro ffmpeg..."
apt-get remove -y ffmpeg || true

echo "[ffmpeg8] Creating work dir..."
mkdir -p /opt/ffmpeg8
cd /opt/ffmpeg8

FFMPEG_ARCHIVE_URL="${FFMPEG_ARCHIVE_URL:-""}"

if [[ -z "${FFMPEG_ARCHIVE_URL}" ]]; then
  echo "ERROR: FFMPEG_ARCHIVE_URL is not set. Please set it to a Linux x86_64 FFmpeg 8.x static build URL."
  exit 1
fi

echo "[ffmpeg8] Downloading ffmpeg archive from: ${FFMPEG_ARCHIVE_URL}"
curl -L "${FFMPEG_ARCHIVE_URL}" -o ffmpeg8.tar.xz

echo "[ffmpeg8] Extracting..."
rm -rf ffmpeg8
mkdir ffmpeg8
tar -xf ffmpeg8.tar.xz -C ffmpeg8 --strip-components=1

echo "[ffmpeg8] Installing to /usr/local/bin..."
install -m 0755 ffmpeg8/ffmpeg /usr/local/bin/ffmpeg
install -m 0755 ffmpeg8/ffprobe /usr/local/bin/ffprobe

echo "[ffmpeg8] Done. Version:"
ffmpeg -version | head -n1
```

Usage:

```bash
sudo FFMPEG_ARCHIVE_URL="https://example.com/ffmpeg-8.x-linux64-static.tar.xz"   scripts/install_ffmpeg8.sh
```

The IDE should plug in a real FFmpeg 8.x static build URL or replace this step with a build-from-source process.

---

## 10. Metrics model for the TUI

Define shared types, e.g. `crates/metrics` or `crates/daemon/src/metrics.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobMetrics {
    pub id: String,
    pub input_path: String,
    pub stage: String,          // "queued", "encoding", "validating", etc.
    pub progress: f32,          // 0.0 - 1.0
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub crf: u8,                // always 8 for this profile
    pub encoder: String,        // "svt-av1"
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage_percent: f32,
    pub mem_usage_percent: f32,
    pub load_avg_1: f32,
    pub load_avg_5: f32,
    pub load_avg_15: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
```

In the daemon, maintain `Arc<RwLock<MetricsSnapshot>>` and:

- Have the job scheduler update queue length / running/completed counts.
- Have a background task using `sysinfo` to update `SystemMetrics` periodically.
- Have the encoding pipeline update `JobMetrics` (progress, fps, bitrate, etc.).

---

## 11. Metrics endpoint in the daemon

Expose `/metrics` over HTTP for the TUI to read.

Add dependencies to `crates/daemon/Cargo.toml` (if not present):

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sysinfo = "0.32"
axum = "0.7"
```

Implement a small server, e.g. in `metrics_server.rs`:

```rust
use std::sync::{Arc, RwLock};
use axum::{routing::get, Router};
use axum::response::IntoResponse;
use crate::metrics::MetricsSnapshot;

async fn metrics_handler(state: Arc<RwLock<MetricsSnapshot>>) -> impl IntoResponse {
    let snapshot = {
        let guard = state.read().unwrap();
        guard.clone()
    };
    axum::Json(snapshot)
}

pub async fn run_metrics_server(
    state: Arc<RwLock<MetricsSnapshot>>,
) -> anyhow::Result<()> {
    let app = Router::new().route(
        "/metrics",
        get({
            let state = state.clone();
            move || metrics_handler(state.clone())
        }),
    );

    let addr = "127.0.0.1:7878".parse().unwrap();
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
```

Spawn this server on startup with `tokio::spawn`.

---

## 12. TUI crate: `av1-dashboard` (Ratatui)

Create `crates/tui/Cargo.toml`:

```toml
[package]
name = "av1-dashboard"
version = "0.1.0"
edition = "2021"

[dependencies]
ratatui = "0.28"
crossterm = "0.27"
sysinfo = "0.32"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

Add to workspace:

```toml
[workspace]
members = [
  "crates/cli-daemon",
  "crates/daemon",
  "third_party/av1an",
  "crates/tui",
]
```

In `crates/tui/src/main.rs`:

- Initialize a terminal using Crossterm + Ratatui.
- Every ~500 ms:
  - Fetch `http://127.0.0.1:7878/metrics` using `reqwest`.
  - Deserialize into `MetricsSnapshot`.
  - Redraw the UI.

Suggested layout:

- **Top-left:** Queue table
  - Columns: ID, Stage, Progress %, FPS, Bitrate, CRF, Workers, ETA.
- **Bottom-left:** Job detail
  - Path, frames, sizes, encoder settings, quality metrics.
- **Top-right:** System gauges
  - CPU% and memory% gauges; load averages in a small table.
- **Middle-right:** Throughput chart
  - MB encoded per minute/hour vs time.
- **Bottom-right:** Event log
  - Last N events (job start/end, failures, gate reasons).

Use Ratatui widgets: `Layout`, `Block`, `Table`, `Gauge`, `Chart`, `Paragraph`, etc.

---

## 13. Av1an log parsing for richer metrics

To fill `fps`, `bitrate_kbps`, `est_remaining_secs`, `frames_encoded`, etc., parse Av1an logs:

1. Run Av1an with `--log-file` (you can extend the command above with `--log-file <path>` if you want, without affecting the core settings).
2. Tail the log file or capture stdout and parse lines for:
   - Current fps, bitrate, ETA.
   - Frames encoded vs total.
3. Update `JobMetrics` accordingly via the shared metrics state.

This parsing is optional but enables **richer metrics** in the TUI.

---

## 14. Build & run on Debian

1. Clone + submodules:

   ```bash
   git clone <your-repo-url>
   cd <your-repo>
   git submodule update --init --recursive
   ```

2. Install FFmpeg 8+:

   ```bash
   sudo FFMPEG_ARCHIVE_URL="https://example.com/ffmpeg-8.x-linux64-static.tar.xz"      scripts/install_ffmpeg8.sh
   ```

3. Install Rust & build:

   ```bash
   cargo build --release
   ```

   Produces:
   - `target/release/cli-daemon`
   - `target/release/av1an`
   - `target/release/av1-dashboard`

4. Run daemon + TUI:

   ```bash
   ./target/release/cli-daemon --config /path/to/config.toml &
   ./target/release/av1-dashboard
   ```

Daemon startup will:

- Enforce software-only settings.
- Ensure `av1an` is available.
- Require FFmpeg 8+.
- Compute concurrency with workers=8 on a 32-core CPU.
- Start the metrics server.

TUI will connect to `/metrics` and render live metrics.

---

## 15. Tests & checklist

### 15.1. Unit tests

- Concurrency planner:
  - For `logical_cores = 32`, ensure `av1an_workers = 8`, `max_concurrent_jobs = 1`.
- Av1an command builder:
  - Assert the built command contains all flags:
    - `--encoder svt-av1`
    - `--pix-format yuv420p10le`
    - `--crf 8`
    - `--preset 3`
    - `--svt-params tune=grain:...`
    - `--target-quality 1`
    - `--audio-copy`
    - `--workers <expected>`
    - `--temp <dir>`
- FFmpeg version checker:
  - Verify it rejects 7.x and accepts 8.x.
- Software safety:
  - Confirm hardware-related flags in any user args are rejected when enabled.

### 15.2. Integration tests

- Run the daemon on a small test library.
- Verify:
  - Outputs are AV1 with yuv420p10le.
  - Durations match originals within tolerance.
  - Size gates and replacement behave as before.

### 15.3. Manual CPU utilization check

- On a 32-core host, run some large encodes.
- Confirm CPU utilization ~80–90% (given `workers=8` and the heavy film-grain settings).

---

## 16. Summary for the AI IDE

This single document now reflects **your exact Av1an settings**:

- `svt-av1`, `yuv420p10le`, `crf=8`, `preset=3`,
- film-grain tuning via `--svt-params "tune=grain:film-grain=20:enable-qm=1:qm-min=1:qm-max=15:keyint=240:lookahead=40"`,
- `--target-quality 1`, `--audio-copy`,
- `--workers 8` (derived from concurrency plan on a 32-core host),
- `--temp "temp_chunks"`.

And wraps them in a full system architecture with:

- Software-only guarantees,
- FFmpeg 8 enforcement,
- A job daemon,
- And a rich TUI exposing as many metrics as the system can produce.
