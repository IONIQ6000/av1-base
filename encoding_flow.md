## Startup (before any file work)
1. Load configuration (`config.toml` or defaults) via `config::load_config`. Ensures sane values (non-empty library roots, `max_size_ratio` in (0,1], at least one concurrent slot).
2. Validate environment:
   - `startup::check_ffmpeg_version` rejects FFmpeg < 8.
   - `startup::detect_available_encoders` scans `ffmpeg -encoders` for `libsvtav1`, `libaom-av1`, `librav1e`.
3. Pick the encoder with `startup::select_encoder`, honoring `prefer_encoder` and falling back `svt -> aom -> rav1e`.
4. Create required directories (`job_state_dir`, `temp_output_dir`).

## Scan Cycle (file detection)
Each cycle of `daemon_loop::run_daemon_loop`:
- Load existing job JSON to avoid duplicate work on in-flight items.
- Walk every `library_root` recursively (`scan::scan_libraries`):
  - Skip hidden directories (except the root), consider only video extensions (`.mkv`, `.mp4`, `.avi`, `.mov`, `.m4v`, `.ts`, `.m2ts`).
  - Ignore files already carrying a `.av1skip` marker.
  - Capture size and modified time for later stability checks.

## Candidate Pipeline (per file)
Processing is serialized per candidate inside a scan cycle (`process_candidate`):
1. **Duplicate job guard**: Skip if a pending/running job already exists for this path.
2. **User skip**: Abort if `.av1skip` is present.
3. **Stability check**: Wait 10 seconds (`stable::check_stability`) and ensure file size is unchanged; unstable files are retried next cycle.
4. **Probe**: Run `ffprobe` to collect streams/format (`probe::probe_file`). On failure, create `.av1skip` and optional `.why.txt`.
5. **Classify source**: `classify::classify_source` labels WebLike/DiscLike/Unknown using path keywords, bitrate vs resolution, codec. WebLike toggles timestamp-safe FFmpeg flags and padding later.
6. **Gates (`gates::check_gates`)**:
   - Skip marker (defensive re-check),
   - No video streams,
   - Smaller than `min_bytes`,
   - Already AV1 on the first video stream.
   Skips create `.av1skip` and optional `.why.txt`.
7. **Job creation**: Persist a JSON job (`jobs::create_job` + `save_job`) with probed metadata (codec, bitrate, dimensions, duration, HDR hints) and classification (`is_web_like`).

## Encoding
1. Mark job `Running`, assign temp output `${temp_output_dir}/{job_id}.mkv`, and store encoding choices:
   - CRF from `encode::select_crf` (height-based; `very_high` tier subtracts 2).
   - SVT preset from `encode::select_preset` (height-based; `very_high` slows by 2, clamped).
   - Encoder codec name from the startup selection.
2. Build the FFmpeg command (`encode::build_command`):
   - Common: `-hide_banner -y`, input path, stream mapping that keeps everything except attachments and Russian audio/subs, copies chapters/metadata.
   - WebLike sources add timestamp-stabilizing flags (`-fflags +genpts -copyts -start_at_zero -vsync 0 -avoid_negative_ts make_zero`).
   - Pad filter `pad=ceil(iw/2)*2:ceil(ih/2)*2,setsar=1` for WebLike or odd dimensions.
   - Encoder-specific:
     - **SVT-AV1**: `-c:v libsvtav1 -crf <crf> -preset <preset> -threads 0 -svtav1-params lp=0`.
     - **libaom-av1**: constant quality (`-b:v 0 -crf <crf>`), `-cpu-used` from resolution (<=1080p:4, >1080p:3), row-mt on, tiles (<=1080p:2x1, >1080p:2x2, >2160p:3x2).
     - **librav1e**: `-qp <crf>` as fallback.
   - Audio/subtitles copied (`-c:a copy -c:s copy`), `-max_muxing_queue_size 2048`, output path last.
3. Execute with progress parsing (`encode::execute_encode`), wrapped in a concurrency gate (`JobExecutor` semaphore capped by `max_concurrent_jobs`):
   - Runs `ffmpeg -progress pipe:1 -nostats ...`.
   - Streams progress to update `encoded_bytes`, `encoded_duration`, `progress`, `eta`, `speed_bps`, `output_est_bytes`, and flips job `stage` from `Encoding` to `Verifying`.
   - Failures capture stderr, mark job `Failed` with reason, and stop.

## Post-Encode Checks
1. **Validation (`validate::validate_output`)**:
   - Re-probe the output.
   - Require exactly one AV1 video stream.
   - Duration must match the original within ±2 seconds.
   On failure: mark `Failed`, record reason, delete the temp output.
2. **Size gate (`size_gate::check_size_gate`)**:
   - Compare output size vs original; reject if `output >= original * max_size_ratio`.
   - On fail: status `Skipped`, reason stored, temp output deleted, `.av1skip` (and `.why.txt` if enabled) created to avoid reprocessing.

## Replacement and Completion
1. **Atomic swap (`replace::atomic_replace`)**:
   - Backs up original to `<name>.orig.<timestamp>` (renames when possible; copy fallback for cross-filesystem/ZFS quirks).
   - Copies the encoded file into place; deletes temp output; optionally deletes backup unless `keep_original` is true.
   - If backup cannot be created, uses a safe copy/delete/rename sequence and preserves temp files on errors.
2. **Finalize job**:
   - On success: job `stage` → `Complete`, `status` → `Success`.
   - On replacement failure: job `Failed` with reason; encoded file is left for manual inspection.

## Sidecars and Persistence
- Every material state change re-saves the job JSON in `job_state_dir`; the TUI consumes these files for live status.
- Skip markers (`.av1skip`) prevent future work on a file. When `write_why_sidecars` is true, a peer `.why.txt` explains the skip cause (probe failure, gate reason, size gate failure).
