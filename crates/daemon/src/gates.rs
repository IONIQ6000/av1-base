//! Gates module for validating video files before encoding.
//!
//! This module provides functionality to probe video files using ffprobe
//! and check various gates (no video streams, minimum size, already AV1)
//! to determine if a file should proceed to encoding.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

/// Error type for probe operations.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// ffprobe command failed to execute.
    #[error("ffprobe failed: {0}")]
    FfprobeFailed(String),

    /// Failed to parse ffprobe JSON output.
    #[error("Failed to parse ffprobe output: {0}")]
    ParseError(String),

    /// IO error during probe.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Information about a video stream from ffprobe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoStream {
    /// Codec name (e.g., "hevc", "h264", "av1").
    pub codec_name: String,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
    /// Bitrate in kbps (if available).
    pub bitrate_kbps: Option<f32>,
}

/// Information about an audio stream from ffprobe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioStream {
    /// Codec name (e.g., "aac", "truehd", "dts").
    pub codec_name: String,
    /// Number of audio channels.
    pub channels: u32,
}

/// Format information from ffprobe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormatInfo {
    /// Duration in seconds.
    pub duration_secs: f64,
    /// File size in bytes.
    pub size_bytes: u64,
}


/// Result of probing a video file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    /// Video streams found in the file.
    pub video_streams: Vec<VideoStream>,
    /// Audio streams found in the file.
    pub audio_streams: Vec<AudioStream>,
    /// Format information.
    pub format: FormatInfo,
}

/// Configuration for gate checks.
#[derive(Debug, Clone)]
pub struct GatesConfig {
    /// Minimum file size in bytes.
    pub min_bytes: u64,
    /// Maximum output/original size ratio (0, 1].
    pub max_size_ratio: f32,
    /// Whether to keep original file after replacement.
    pub keep_original: bool,
}

impl Default for GatesConfig {
    fn default() -> Self {
        Self {
            min_bytes: 1048576, // 1 MB
            max_size_ratio: 0.95,
            keep_original: false,
        }
    }
}

/// Result of gate checking.
#[derive(Debug, Clone, PartialEq)]
pub enum GateResult {
    /// File passed all gates and can proceed to encoding.
    Pass(ProbeResult),
    /// File should be skipped with the given reason.
    Skip { reason: String },
}

/// Raw ffprobe JSON structures for parsing.
mod ffprobe_json {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct FfprobeOutput {
        pub streams: Option<Vec<Stream>>,
        pub format: Option<Format>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Stream {
        pub codec_type: Option<String>,
        pub codec_name: Option<String>,
        pub width: Option<u32>,
        pub height: Option<u32>,
        pub bit_rate: Option<String>,
        pub channels: Option<u32>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Format {
        pub duration: Option<String>,
        pub size: Option<String>,
    }
}


/// Probes a video file using ffprobe to collect stream and format metadata.
///
/// Runs `ffprobe -v quiet -print_format json -show_streams -show_format <path>`
/// and parses the JSON output.
pub fn probe_file(path: &Path) -> Result<ProbeResult, ProbeError> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ProbeError::FfprobeFailed(format!(
            "ffprobe exited with status {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ffprobe_output(&stdout)
}

/// Parses ffprobe JSON output into a ProbeResult.
pub fn parse_ffprobe_output(json_str: &str) -> Result<ProbeResult, ProbeError> {
    let ffprobe: ffprobe_json::FfprobeOutput =
        serde_json::from_str(json_str).map_err(|e| ProbeError::ParseError(e.to_string()))?;

    let streams = ffprobe.streams.unwrap_or_default();
    let format = ffprobe.format.ok_or_else(|| {
        ProbeError::ParseError("Missing format information in ffprobe output".to_string())
    })?;

    let mut video_streams = Vec::new();
    let mut audio_streams = Vec::new();

    for stream in streams {
        let codec_type = stream.codec_type.as_deref().unwrap_or("");
        let codec_name = stream.codec_name.clone().unwrap_or_default();

        match codec_type {
            "video" => {
                let bitrate_kbps = stream
                    .bit_rate
                    .as_ref()
                    .and_then(|br| br.parse::<f64>().ok())
                    .map(|bps| (bps / 1000.0) as f32);

                video_streams.push(VideoStream {
                    codec_name,
                    width: stream.width.unwrap_or(0),
                    height: stream.height.unwrap_or(0),
                    bitrate_kbps,
                });
            }
            "audio" => {
                audio_streams.push(AudioStream {
                    codec_name,
                    channels: stream.channels.unwrap_or(0),
                });
            }
            _ => {}
        }
    }

    let duration_secs = format
        .duration
        .as_ref()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    let size_bytes = format
        .size
        .as_ref()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(ProbeResult {
        video_streams,
        audio_streams,
        format: FormatInfo {
            duration_secs,
            size_bytes,
        },
    })
}


/// Checks if a file passes all gates for encoding.
///
/// Gates checked:
/// 1. No video streams -> skip with "no video streams"
/// 2. File size < min_bytes -> skip with "below minimum size"
/// 3. First video stream is AV1 -> skip with "already AV1"
///
/// Returns `GateResult::Pass` with the probe result if all gates pass.
pub fn check_gates(probe: &ProbeResult, file_size: u64, cfg: &GatesConfig) -> GateResult {
    // Gate 1: Check for no video streams
    if probe.video_streams.is_empty() {
        return GateResult::Skip {
            reason: "no video streams".to_string(),
        };
    }

    // Gate 2: Check minimum file size
    if file_size < cfg.min_bytes {
        return GateResult::Skip {
            reason: format!(
                "below minimum size ({} bytes < {} bytes)",
                file_size, cfg.min_bytes
            ),
        };
    }

    // Gate 3: Check if first video stream is already AV1
    if let Some(first_video) = probe.video_streams.first() {
        if first_video.codec_name.to_lowercase().contains("av1") {
            return GateResult::Skip {
                reason: "already AV1".to_string(),
            };
        }
    }

    // All gates passed
    GateResult::Pass(probe.clone())
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
    fn make_probe_result(video_streams: Vec<VideoStream>, audio_streams: Vec<AudioStream>) -> ProbeResult {
        ProbeResult {
            video_streams,
            audio_streams,
            format: FormatInfo {
                duration_secs: 3600.0,
                size_bytes: 5_000_000_000,
            },
        }
    }

    // **Feature: av1-super-daemon, Property 13: Gate Rejection for No Video Streams**
    // **Validates: Requirements 13.3**
    //
    // *For any* probe result with zero video streams, the gate checker SHALL return
    // `Skip` with reason containing "no video streams".
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_gate_rejection_no_video_streams(
            num_audio_streams in 0usize..5,
            file_size in 1_000_000u64..10_000_000_000,
            min_bytes in 1u64..1_000_000,
        ) {
            // Create probe result with NO video streams
            let audio_streams: Vec<AudioStream> = (0..num_audio_streams)
                .map(|i| make_audio_stream(&format!("aac{}", i), 2))
                .collect();

            let probe = ProbeResult {
                video_streams: vec![], // No video streams
                audio_streams,
                format: FormatInfo {
                    duration_secs: 3600.0,
                    size_bytes: file_size,
                },
            };

            let cfg = GatesConfig {
                min_bytes,
                max_size_ratio: 0.95,
                keep_original: false,
            };

            let result = check_gates(&probe, file_size, &cfg);

            // Should always be Skip with "no video streams" reason
            match result {
                GateResult::Skip { reason } => {
                    prop_assert!(
                        reason.contains("no video streams"),
                        "Skip reason should contain 'no video streams', got: {}",
                        reason
                    );
                }
                GateResult::Pass(_) => {
                    prop_assert!(false, "Should not pass gate with no video streams");
                }
            }
        }
    }


    // **Feature: av1-super-daemon, Property 14: Gate Rejection for Minimum Size**
    // **Validates: Requirements 13.4**
    //
    // *For any* file size below the configured `min_bytes` threshold, the gate checker
    // SHALL return `Skip` with reason containing "below minimum size".
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_gate_rejection_minimum_size(
            min_bytes in 1_000u64..10_000_000,
            // File size is strictly less than min_bytes
            file_size_offset in 1u64..1000,
            codec in "[a-z]{3,6}",
        ) {
            // Ensure codec is not av1
            prop_assume!(!codec.to_lowercase().contains("av1"));

            let file_size = min_bytes.saturating_sub(file_size_offset);
            // Ensure file_size is actually less than min_bytes
            prop_assume!(file_size < min_bytes);

            let probe = make_probe_result(
                vec![make_video_stream(&codec, 1920, 1080)],
                vec![make_audio_stream("aac", 2)],
            );

            let cfg = GatesConfig {
                min_bytes,
                max_size_ratio: 0.95,
                keep_original: false,
            };

            let result = check_gates(&probe, file_size, &cfg);

            match result {
                GateResult::Skip { reason } => {
                    prop_assert!(
                        reason.contains("below minimum size"),
                        "Skip reason should contain 'below minimum size', got: {}",
                        reason
                    );
                }
                GateResult::Pass(_) => {
                    prop_assert!(
                        false,
                        "Should not pass gate with file_size {} < min_bytes {}",
                        file_size, min_bytes
                    );
                }
            }
        }
    }

    // **Feature: av1-super-daemon, Property 15: Gate Rejection for Already AV1**
    // **Validates: Requirements 13.5**
    //
    // *For any* probe result where the first video stream has codec name containing "av1"
    // (case-insensitive), the gate checker SHALL return `Skip` with reason containing "already AV1".
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_gate_rejection_already_av1(
            // Generate various AV1 codec name variations
            av1_variant in prop_oneof![
                Just("av1".to_string()),
                Just("AV1".to_string()),
                Just("Av1".to_string()),
                Just("av1_nvenc".to_string()),
                Just("libaom-av1".to_string()),
                Just("libsvtav1".to_string()),
                Just("av1_qsv".to_string()),
            ],
            file_size in 10_000_000u64..100_000_000_000,
            min_bytes in 1u64..1_000_000,
        ) {
            let probe = make_probe_result(
                vec![make_video_stream(&av1_variant, 1920, 1080)],
                vec![make_audio_stream("aac", 2)],
            );

            let cfg = GatesConfig {
                min_bytes,
                max_size_ratio: 0.95,
                keep_original: false,
            };

            let result = check_gates(&probe, file_size, &cfg);

            match result {
                GateResult::Skip { reason } => {
                    prop_assert!(
                        reason.contains("already AV1"),
                        "Skip reason should contain 'already AV1', got: {}",
                        reason
                    );
                }
                GateResult::Pass(_) => {
                    prop_assert!(
                        false,
                        "Should not pass gate with AV1 codec: {}",
                        av1_variant
                    );
                }
            }
        }
    }


    // **Feature: av1-super-daemon, Property 16: Gate Pass for Valid Files**
    // **Validates: Requirements 13.6**
    //
    // *For any* probe result with at least one non-AV1 video stream, file size >= `min_bytes`,
    // the gate checker SHALL return `Pass`.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_gate_pass_valid_files(
            // Non-AV1 codec names
            codec in prop_oneof![
                Just("hevc".to_string()),
                Just("h264".to_string()),
                Just("h265".to_string()),
                Just("mpeg4".to_string()),
                Just("vp9".to_string()),
                Just("mpeg2video".to_string()),
                Just("prores".to_string()),
            ],
            min_bytes in 1u64..1_000_000,
            // File size is >= min_bytes
            file_size_offset in 0u64..100_000_000,
            num_audio_streams in 0usize..5,
        ) {
            let file_size = min_bytes + file_size_offset;

            let audio_streams: Vec<AudioStream> = (0..num_audio_streams)
                .map(|i| make_audio_stream(&format!("aac{}", i), 2))
                .collect();

            let probe = ProbeResult {
                video_streams: vec![make_video_stream(&codec, 1920, 1080)],
                audio_streams,
                format: FormatInfo {
                    duration_secs: 3600.0,
                    size_bytes: file_size,
                },
            };

            let cfg = GatesConfig {
                min_bytes,
                max_size_ratio: 0.95,
                keep_original: false,
            };

            let result = check_gates(&probe, file_size, &cfg);

            match result {
                GateResult::Pass(returned_probe) => {
                    // Verify the returned probe matches the input
                    prop_assert_eq!(
                        returned_probe.video_streams.len(),
                        probe.video_streams.len(),
                        "Returned probe should have same number of video streams"
                    );
                    prop_assert_eq!(
                        &returned_probe.video_streams[0].codec_name,
                        &codec,
                        "Returned probe should have same codec"
                    );
                }
                GateResult::Skip { reason } => {
                    prop_assert!(
                        false,
                        "Valid file should pass gates, but got Skip: {} (codec={}, file_size={}, min_bytes={})",
                        reason, codec, file_size, min_bytes
                    );
                }
            }
        }
    }

    // Unit tests for ffprobe JSON parsing
    #[test]
    fn test_parse_ffprobe_output_basic() {
        let json = r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "hevc",
                    "width": 1920,
                    "height": 1080,
                    "bit_rate": "25000000"
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac",
                    "channels": 6
                }
            ],
            "format": {
                "duration": "7200.5",
                "size": "22548578304"
            }
        }"#;

        let result = parse_ffprobe_output(json).expect("Should parse valid JSON");

        assert_eq!(result.video_streams.len(), 1);
        assert_eq!(result.video_streams[0].codec_name, "hevc");
        assert_eq!(result.video_streams[0].width, 1920);
        assert_eq!(result.video_streams[0].height, 1080);
        assert!((result.video_streams[0].bitrate_kbps.unwrap() - 25000.0).abs() < 0.1);

        assert_eq!(result.audio_streams.len(), 1);
        assert_eq!(result.audio_streams[0].codec_name, "aac");
        assert_eq!(result.audio_streams[0].channels, 6);

        assert!((result.format.duration_secs - 7200.5).abs() < 0.001);
        assert_eq!(result.format.size_bytes, 22548578304);
    }

    #[test]
    fn test_parse_ffprobe_output_no_streams() {
        let json = r#"{
            "streams": [],
            "format": {
                "duration": "100.0",
                "size": "1000000"
            }
        }"#;

        let result = parse_ffprobe_output(json).expect("Should parse JSON with no streams");
        assert!(result.video_streams.is_empty());
        assert!(result.audio_streams.is_empty());
    }

    #[test]
    fn test_parse_ffprobe_output_missing_optional_fields() {
        let json = r#"{
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264"
                }
            ],
            "format": {
                "duration": "60.0",
                "size": "500000"
            }
        }"#;

        let result = parse_ffprobe_output(json).expect("Should parse JSON with missing optional fields");
        assert_eq!(result.video_streams.len(), 1);
        assert_eq!(result.video_streams[0].width, 0);
        assert_eq!(result.video_streams[0].height, 0);
        assert!(result.video_streams[0].bitrate_kbps.is_none());
    }

    #[test]
    fn test_check_gates_no_video_streams() {
        let probe = make_probe_result(vec![], vec![make_audio_stream("aac", 2)]);
        let cfg = GatesConfig::default();

        let result = check_gates(&probe, 10_000_000, &cfg);
        match result {
            GateResult::Skip { reason } => {
                assert!(reason.contains("no video streams"));
            }
            _ => panic!("Expected Skip result"),
        }
    }

    #[test]
    fn test_check_gates_below_min_size() {
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 1920, 1080)],
            vec![],
        );
        let cfg = GatesConfig {
            min_bytes: 10_000_000,
            ..Default::default()
        };

        let result = check_gates(&probe, 5_000_000, &cfg);
        match result {
            GateResult::Skip { reason } => {
                assert!(reason.contains("below minimum size"));
            }
            _ => panic!("Expected Skip result"),
        }
    }

    #[test]
    fn test_check_gates_already_av1() {
        let probe = make_probe_result(
            vec![make_video_stream("av1", 1920, 1080)],
            vec![],
        );
        let cfg = GatesConfig {
            min_bytes: 1_000,
            ..Default::default()
        };

        let result = check_gates(&probe, 10_000_000, &cfg);
        match result {
            GateResult::Skip { reason } => {
                assert!(reason.contains("already AV1"));
            }
            _ => panic!("Expected Skip result"),
        }
    }

    #[test]
    fn test_check_gates_pass() {
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 1920, 1080)],
            vec![make_audio_stream("aac", 2)],
        );
        let cfg = GatesConfig {
            min_bytes: 1_000,
            ..Default::default()
        };

        let result = check_gates(&probe, 10_000_000, &cfg);
        match result {
            GateResult::Pass(returned_probe) => {
                assert_eq!(returned_probe.video_streams[0].codec_name, "hevc");
            }
            _ => panic!("Expected Pass result"),
        }
    }
}
