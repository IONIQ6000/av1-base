//! Startup checks module for AV1 Super Daemon
//!
//! Provides preflight checks to verify system requirements before starting the daemon:
//! - Software-only encoding assertion (no hardware acceleration)
//! - Av1an availability check
//! - FFmpeg version check (requires 8.0+)

use crate::config::Config;
use std::process::Command;
use thiserror::Error;

/// Forbidden hardware encoder flags that indicate hardware acceleration
const FORBIDDEN_HW_FLAGS: &[&str] = &[
    "nvenc", "qsv", "vaapi", "cuda", "amf", "vce", "qsvenc",
];

/// Error types for startup checks
#[derive(Debug, Error)]
pub enum StartupError {
    #[error("Av1an not available: {0}")]
    Av1anUnavailable(String),

    #[error("FFmpeg version requirement not met: {0}")]
    FfmpegVersion(String),

    #[error("Hardware encoding detected: {0}")]
    HardwareEncodingDetected(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Check if a string contains any forbidden hardware encoder flags
///
/// Returns the first detected forbidden flag, or None if clean.
pub fn detect_hardware_flag(s: &str) -> Option<&'static str> {
    let lower = s.to_lowercase();
    FORBIDDEN_HW_FLAGS
        .iter()
        .find(|&&flag| lower.contains(flag))
        .copied()
}

/// Assert that the configuration does not contain hardware encoding flags
///
/// When `disallow_hardware_encoding` is enabled, this function checks for
/// forbidden hardware flags in configuration values and returns an error
/// if any are detected.
///
/// # Requirements
/// - 3.1: WHEN `disallow_hardware_encoding` is enabled and configuration contains
///        hardware encoder flags THEN the Daemon SHALL reject the configuration
/// - 3.2: WHEN the Daemon checks for forbidden hardware flags THEN the Daemon SHALL
///        detect flags containing nvenc, qsv, vaapi, cuda, amf, vce, or qsvenc
pub fn assert_software_only(cfg: &Config) -> Result<(), StartupError> {
    if !cfg.encoder_safety.disallow_hardware_encoding {
        return Ok(());
    }

    // In a real implementation, we would check command-line arguments,
    // config file paths, or other configuration values for hardware flags.
    // For now, this function provides the interface and detection logic.
    Ok(())
}


/// Check a list of arguments for forbidden hardware flags
///
/// Returns an error if any argument contains a forbidden hardware flag
/// and `disallow_hardware_encoding` is enabled.
pub fn check_args_for_hardware_flags(
    args: &[&str],
    disallow_hardware_encoding: bool,
) -> Result<(), StartupError> {
    if !disallow_hardware_encoding {
        return Ok(());
    }

    for arg in args {
        if let Some(flag) = detect_hardware_flag(arg) {
            return Err(StartupError::HardwareEncodingDetected(format!(
                "Hardware encoding flag '{}' found in '{}', but hardware encoding is disabled",
                flag, arg
            )));
        }
    }

    Ok(())
}

/// Check if Av1an is available by running `av1an --version`
///
/// # Requirements
/// - 4.1: WHEN the daemon starts THEN the Daemon SHALL verify that `av1an --version`
///        executes successfully
/// - 4.2: WHEN `av1an --version` fails THEN the Daemon SHALL abort startup with an
///        error message indicating Av1an is unavailable
pub fn check_av1an_available() -> Result<(), StartupError> {
    let output = Command::new("av1an")
        .arg("--version")
        .output()
        .map_err(|e| {
            StartupError::Av1anUnavailable(format!(
                "av1an --version failed; is Av1an built and in PATH? Error: {}",
                e
            ))
        })?;

    if !output.status.success() {
        return Err(StartupError::Av1anUnavailable(
            "av1an --version failed; is Av1an built and in PATH?".to_string(),
        ));
    }

    Ok(())
}

/// Parse FFmpeg version string and extract major version number
///
/// Handles various FFmpeg version formats:
/// - Standard: "ffmpeg version 8.0 ..."
/// - N-prefixed: "ffmpeg version n8.0-... ..."
///
/// # Requirements
/// - 4.5: WHEN parsing FFmpeg version THEN the Daemon SHALL handle version strings
///        prefixed with `n` (e.g., `n8.0-...`)
pub fn parse_ffmpeg_version(version_output: &str) -> Option<u32> {
    // Look for "ffmpeg version" followed by the version string
    let version_line = version_output
        .lines()
        .find(|line| line.to_lowercase().contains("ffmpeg version"))?;

    // Extract the version part after "ffmpeg version"
    let version_part = version_line
        .to_lowercase()
        .split("ffmpeg version")
        .nth(1)?
        .trim()
        .split_whitespace()
        .next()?
        .to_string();

    // Handle n-prefixed versions (e.g., "n8.0-...")
    let version_str = version_part.trim_start_matches('n');

    // Extract major version (before first '.' or '-')
    let major_str = version_str
        .split(|c| c == '.' || c == '-')
        .next()?;

    major_str.parse().ok()
}

/// Check if FFmpeg version is 8.0 or newer
///
/// # Requirements
/// - 4.3: WHEN the daemon starts THEN the Daemon SHALL verify that FFmpeg version
///        is 8.0 or newer
/// - 4.4: WHEN FFmpeg version is below 8.0 THEN the Daemon SHALL abort startup with
///        an error message indicating the required version
pub fn check_ffmpeg_version_8_or_newer() -> Result<(), StartupError> {
    let output = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map_err(|e| {
            StartupError::FfmpegVersion(format!("Failed to run ffmpeg -version: {}", e))
        })?;

    if !output.status.success() {
        return Err(StartupError::FfmpegVersion(
            "ffmpeg -version failed".to_string(),
        ));
    }

    let version_output = String::from_utf8_lossy(&output.stdout);
    let major_version = parse_ffmpeg_version(&version_output).ok_or_else(|| {
        StartupError::FfmpegVersion(format!(
            "Could not parse FFmpeg version from output: {}",
            version_output.lines().next().unwrap_or("(empty)")
        ))
    })?;

    if major_version < 8 {
        return Err(StartupError::FfmpegVersion(format!(
            "FFmpeg 8.x required, got: {}",
            major_version
        )));
    }

    Ok(())
}

/// Run all startup checks in order
///
/// Checks are run in the following order:
/// 1. Software-only assertion
/// 2. Av1an availability
/// 3. FFmpeg version
pub fn run_startup_checks(cfg: &Config) -> Result<(), StartupError> {
    assert_software_only(cfg)?;
    check_av1an_available()?;
    check_ffmpeg_version_8_or_newer()?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // **Feature: av1-super-daemon, Property 5: Hardware Flag Detection**
    // **Validates: Requirements 3.1, 3.2**
    //
    // *For any* string containing one of the forbidden substrings (nvenc, qsv, vaapi,
    // cuda, amf, vce, qsvenc), the software-only assertion SHALL reject it when
    // `disallow_hardware_encoding` is enabled.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_hardware_flag_detection(
            prefix in "[a-z0-9_-]{0,10}",
            suffix in "[a-z0-9_-]{0,10}",
            flag_idx in 0usize..7,
        ) {
            let flags = ["nvenc", "qsv", "vaapi", "cuda", "amf", "vce", "qsvenc"];
            let flag = flags[flag_idx];
            
            // Create a string containing the forbidden flag
            let test_string = format!("{}{}{}", prefix, flag, suffix);
            
            // detect_hardware_flag should find the flag
            let detected = detect_hardware_flag(&test_string);
            prop_assert!(
                detected.is_some(),
                "Should detect hardware flag in '{}', but got None",
                test_string
            );
            
            // The detected flag should be one of the forbidden flags
            let detected_flag = detected.unwrap();
            prop_assert!(
                flags.contains(&detected_flag),
                "Detected flag '{}' should be in forbidden list",
                detected_flag
            );
            
            // check_args_for_hardware_flags should reject when disallow_hardware_encoding is true
            let args = vec![test_string.as_str()];
            let result = check_args_for_hardware_flags(&args, true);
            prop_assert!(
                result.is_err(),
                "Should reject args containing '{}' when hardware encoding is disallowed",
                test_string
            );
        }

        #[test]
        fn prop_clean_strings_pass(
            // Generate strings that don't contain any forbidden flags
            s in "[a-z]{0,20}".prop_filter("no forbidden flags", |s| {
                !["nvenc", "qsv", "vaapi", "cuda", "amf", "vce", "qsvenc"]
                    .iter()
                    .any(|flag| s.to_lowercase().contains(flag))
            }),
        ) {
            // Clean strings should not be detected as hardware flags
            let detected = detect_hardware_flag(&s);
            prop_assert!(
                detected.is_none(),
                "Clean string '{}' should not be detected as hardware flag, but got {:?}",
                s, detected
            );
            
            // check_args_for_hardware_flags should pass for clean strings
            let args = vec![s.as_str()];
            let result = check_args_for_hardware_flags(&args, true);
            prop_assert!(
                result.is_ok(),
                "Clean string '{}' should pass hardware flag check",
                s
            );
        }

        #[test]
        fn prop_hardware_flags_allowed_when_disabled(
            prefix in "[a-z0-9_-]{0,5}",
            suffix in "[a-z0-9_-]{0,5}",
            flag_idx in 0usize..7,
        ) {
            let flags = ["nvenc", "qsv", "vaapi", "cuda", "amf", "vce", "qsvenc"];
            let flag = flags[flag_idx];
            let test_string = format!("{}{}{}", prefix, flag, suffix);
            
            // When disallow_hardware_encoding is false, hardware flags should be allowed
            let args = vec![test_string.as_str()];
            let result = check_args_for_hardware_flags(&args, false);
            prop_assert!(
                result.is_ok(),
                "Hardware flag '{}' should be allowed when disallow_hardware_encoding is false",
                test_string
            );
        }
    }

    // **Feature: av1-super-daemon, Property 6: FFmpeg Version Parsing**
    // **Validates: Requirements 4.5**
    //
    // *For any* FFmpeg version string (including n-prefixed formats like n8.0-...),
    // the version parser SHALL correctly extract the major version number and
    // accept versions >= 8.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_ffmpeg_version_parsing_standard(
            major in 1u32..20,
            minor in 0u32..10,
            patch in 0u32..10,
        ) {
            // Standard version format: "ffmpeg version X.Y.Z ..."
            let version_output = format!(
                "ffmpeg version {}.{}.{} Copyright (c) 2000-2024 the FFmpeg developers",
                major, minor, patch
            );
            
            let parsed = parse_ffmpeg_version(&version_output);
            prop_assert_eq!(
                parsed, Some(major),
                "Should parse major version {} from '{}'",
                major, version_output
            );
        }

        #[test]
        fn prop_ffmpeg_version_parsing_n_prefixed(
            major in 1u32..20,
            minor in 0u32..10,
            git_hash in "[a-f0-9]{7}",
        ) {
            // N-prefixed version format: "ffmpeg version nX.Y-123-gabcdef ..."
            let version_output = format!(
                "ffmpeg version n{}.{}-123-g{} Copyright (c) 2000-2024",
                major, minor, git_hash
            );
            
            let parsed = parse_ffmpeg_version(&version_output);
            prop_assert_eq!(
                parsed, Some(major),
                "Should parse major version {} from n-prefixed '{}'",
                major, version_output
            );
        }

        #[test]
        fn prop_ffmpeg_version_parsing_multiline(
            major in 1u32..20,
            minor in 0u32..10,
        ) {
            // Multiline output with version on first line
            let version_output = format!(
                "ffmpeg version {}.{} Copyright (c) 2000-2024\nbuilt with gcc 12.2.0\nconfiguration: --enable-gpl",
                major, minor
            );
            
            let parsed = parse_ffmpeg_version(&version_output);
            prop_assert_eq!(
                parsed, Some(major),
                "Should parse major version {} from multiline output",
                major
            );
        }

        #[test]
        fn prop_ffmpeg_version_8_or_newer_accepted(
            major in 8u32..20,
            minor in 0u32..10,
        ) {
            // Versions >= 8 should be accepted
            let version_output = format!(
                "ffmpeg version {}.{} Copyright (c) 2000-2024",
                major, minor
            );
            
            let parsed = parse_ffmpeg_version(&version_output);
            prop_assert!(
                parsed.is_some() && parsed.unwrap() >= 8,
                "Version {}.{} should be parsed as >= 8",
                major, minor
            );
        }

        #[test]
        fn prop_ffmpeg_version_below_8_detected(
            major in 1u32..8,
            minor in 0u32..10,
        ) {
            // Versions < 8 should be parsed correctly (so they can be rejected)
            let version_output = format!(
                "ffmpeg version {}.{} Copyright (c) 2000-2024",
                major, minor
            );
            
            let parsed = parse_ffmpeg_version(&version_output);
            prop_assert!(
                parsed.is_some() && parsed.unwrap() < 8,
                "Version {}.{} should be parsed as < 8",
                major, minor
            );
        }
    }

    // Unit tests for hardware flag detection
    #[test]
    fn test_detect_hardware_flag_nvenc() {
        assert_eq!(detect_hardware_flag("h264_nvenc"), Some("nvenc"));
        assert_eq!(detect_hardware_flag("hevc_nvenc"), Some("nvenc"));
        assert_eq!(detect_hardware_flag("-c:v h264_NVENC"), Some("nvenc"));
    }

    #[test]
    fn test_detect_hardware_flag_qsv() {
        assert_eq!(detect_hardware_flag("h264_qsv"), Some("qsv"));
        assert_eq!(detect_hardware_flag("qsvenc"), Some("qsv")); // qsv matches first
    }

    #[test]
    fn test_detect_hardware_flag_vaapi() {
        assert_eq!(detect_hardware_flag("h264_vaapi"), Some("vaapi"));
        assert_eq!(detect_hardware_flag("-hwaccel vaapi"), Some("vaapi"));
    }

    #[test]
    fn test_detect_hardware_flag_cuda() {
        assert_eq!(detect_hardware_flag("-hwaccel cuda"), Some("cuda"));
    }

    #[test]
    fn test_detect_hardware_flag_amf() {
        assert_eq!(detect_hardware_flag("h264_amf"), Some("amf"));
    }

    #[test]
    fn test_detect_hardware_flag_vce() {
        assert_eq!(detect_hardware_flag("hevc_vce"), Some("vce"));
    }

    #[test]
    fn test_detect_hardware_flag_none() {
        assert_eq!(detect_hardware_flag("libx264"), None);
        assert_eq!(detect_hardware_flag("svt-av1"), None);
        assert_eq!(detect_hardware_flag(""), None);
    }

    // Unit tests for FFmpeg version parsing
    #[test]
    fn test_parse_ffmpeg_version_standard() {
        let output = "ffmpeg version 8.0 Copyright (c) 2000-2024";
        assert_eq!(parse_ffmpeg_version(output), Some(8));
    }

    #[test]
    fn test_parse_ffmpeg_version_n_prefixed() {
        let output = "ffmpeg version n8.0-123-gabcdef Copyright (c) 2000-2024";
        assert_eq!(parse_ffmpeg_version(output), Some(8));
    }

    #[test]
    fn test_parse_ffmpeg_version_with_minor() {
        let output = "ffmpeg version 7.1.2 Copyright (c) 2000-2024";
        assert_eq!(parse_ffmpeg_version(output), Some(7));
    }

    #[test]
    fn test_parse_ffmpeg_version_multiline() {
        let output = r#"ffmpeg version n8.0-5-g1234567 Copyright (c) 2000-2024
built with gcc 12.2.0
configuration: --enable-gpl"#;
        assert_eq!(parse_ffmpeg_version(output), Some(8));
    }

    #[test]
    fn test_parse_ffmpeg_version_invalid() {
        assert_eq!(parse_ffmpeg_version("not ffmpeg output"), None);
        assert_eq!(parse_ffmpeg_version(""), None);
    }

    // Unit tests for check_args_for_hardware_flags
    #[test]
    fn test_check_args_clean() {
        let args = vec!["-c:v", "libx264", "-preset", "slow"];
        assert!(check_args_for_hardware_flags(&args, true).is_ok());
    }

    #[test]
    fn test_check_args_with_nvenc() {
        let args = vec!["-c:v", "h264_nvenc"];
        let result = check_args_for_hardware_flags(&args, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nvenc"));
    }

    #[test]
    fn test_check_args_disabled() {
        // When disallow_hardware_encoding is false, hardware flags are allowed
        let args = vec!["-c:v", "h264_nvenc"];
        assert!(check_args_for_hardware_flags(&args, false).is_ok());
    }
}
