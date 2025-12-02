//! Av1an encoder module for AV1 Super Daemon
//!
//! Provides functionality to build and execute Av1an encoding commands
//! with fixed film-grain-tuned settings.

use crate::ConcurrencyPlan;
use std::path::PathBuf;
use std::process::Command;
use thiserror::Error;

/// Fixed SVT-AV1 parameters for film-grain tuning
/// Includes CRF, preset, and film-grain settings for the encoder
/// tune: 0=VQ, 1=PSNR, 2=SSIM (no tune 3 in newer SVT-AV1)
const SVT_PARAMS: &str = "--crf 8 --preset 3 --film-grain 20 --enable-qm 1 --qm-min 1 --qm-max 15 --keyint 240 --lookahead 40";

/// Error type for encoding operations
#[derive(Debug, Error)]
pub enum EncodeError {
    /// Av1an process exited with non-zero status
    #[error("Av1an failed with exit code: {0}")]
    Av1anFailed(i32),

    /// Av1an process was terminated by signal
    #[error("Av1an process was terminated by signal")]
    Av1anTerminated,

    /// IO error during encoding
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Parameters for an Av1an encoding job
///
/// Contains all necessary information to execute an encoding job.
#[derive(Debug, Clone)]
pub struct Av1anEncodeParams {
    /// Path to the input video file
    pub input_path: PathBuf,
    /// Path for the encoded output file
    pub output_path: PathBuf,
    /// Directory for temporary chunk files during encoding
    pub temp_chunks_dir: PathBuf,
    /// Concurrency settings for the encoding job
    pub concurrency: ConcurrencyPlan,
}

impl Av1anEncodeParams {
    /// Create new encoding parameters
    pub fn new(
        input_path: PathBuf,
        output_path: PathBuf,
        temp_chunks_dir: PathBuf,
        concurrency: ConcurrencyPlan,
    ) -> Self {
        Self {
            input_path,
            output_path,
            temp_chunks_dir,
            concurrency,
        }
    }
}


/// Build an Av1an command with all required encoding flags
///
/// Creates a Command configured with:
/// - Input and output paths
/// - SVT-AV1 encoder with film-grain tuning
/// - Fixed quality settings (CRF 8, preset 3, yuv420p10le)
/// - Worker count from concurrency plan
/// - Temporary directory for chunks
///
/// # Arguments
/// * `params` - Encoding parameters including paths and concurrency settings
///
/// # Returns
/// A configured Command ready for execution
pub fn build_av1an_command(params: &Av1anEncodeParams) -> Command {
    let mut cmd = Command::new("av1an");

    // Input and output paths (Requirements 10.1, 10.2)
    cmd.arg("-i").arg(&params.input_path);
    cmd.arg("-o").arg(&params.output_path);

    // Encoder selection (Requirements 2.1, 10.3)
    cmd.arg("--encoder").arg("svt-av1");

    // Pixel format (Requirements 2.2, 10.4)
    cmd.arg("--pix-format").arg("yuv420p10le");

    // Video encoder parameters including CRF, preset, and film-grain tuning
    // (Requirements 2.3, 2.4, 2.5, 10.5, 10.6, 10.7)
    cmd.arg("--video-params").arg(SVT_PARAMS);

    // Audio handling - copy all audio streams (Requirements 2.7, 10.9)
    cmd.arg("--audio-params").arg("-c:a copy");

    // Worker count from concurrency plan (Requirements 10.10)
    cmd.arg("--workers")
        .arg(params.concurrency.av1an_workers.to_string());

    // Temporary chunks directory (Requirements 10.11)
    cmd.arg("--temp").arg(&params.temp_chunks_dir);

    cmd
}


/// Execute an Av1an encoding job
///
/// Builds and runs the Av1an command, handling exit status appropriately.
///
/// # Arguments
/// * `params` - Encoding parameters for the job
///
/// # Returns
/// * `Ok(())` - Encoding completed successfully
/// * `Err(EncodeError)` - Encoding failed
///
/// # Errors
/// Returns an error if:
/// - The Av1an process fails to start (IO error)
/// - The Av1an process exits with non-zero status
/// - The Av1an process is terminated by a signal
pub fn run_av1an(params: &Av1anEncodeParams) -> Result<(), EncodeError> {
    let mut cmd = build_av1an_command(params);

    let status = cmd.status()?;

    if status.success() {
        Ok(())
    } else {
        match status.code() {
            Some(code) => Err(EncodeError::Av1anFailed(code)),
            None => Err(EncodeError::Av1anTerminated),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::ffi::OsStr;

    /// Helper to convert Command args to a Vec of strings for easier testing
    fn get_command_args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .filter_map(|arg| arg.to_str().map(String::from))
            .collect()
    }

    /// Helper to check if args contain a flag with a specific value
    fn has_flag_with_value(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    }

    /// Helper to check if args contain a standalone flag
    fn has_flag(args: &[String], flag: &str) -> bool {
        args.iter().any(|arg| arg == flag)
    }

    // Strategy for generating valid path-like strings
    fn path_strategy() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-zA-Z0-9_/.-]{1,50}")
            .unwrap()
            .prop_filter("non-empty path", |s| !s.is_empty())
    }

    // **Feature: av1-super-daemon, Property 4: Av1an Command Completeness**
    // **Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 10.1, 10.2, 10.3, 10.4, 10.5, 10.6, 10.7, 10.8, 10.9, 10.10, 10.11**
    //
    // *For any* valid `Av1anEncodeParams` (input path, output path, temp dir, concurrency plan),
    // the built command SHALL contain all required arguments.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_av1an_command_completeness(
            input_path in path_strategy(),
            output_path in path_strategy(),
            temp_dir in path_strategy(),
            total_cores in 1u32..256,
            av1an_workers in 1u32..64,
            max_concurrent_jobs in 1u32..16,
        ) {
            let concurrency = ConcurrencyPlan {
                total_cores,
                target_threads: total_cores,
                av1an_workers,
                max_concurrent_jobs,
            };

            let params = Av1anEncodeParams::new(
                PathBuf::from(&input_path),
                PathBuf::from(&output_path),
                PathBuf::from(&temp_dir),
                concurrency,
            );

            let cmd = build_av1an_command(&params);
            let args = get_command_args(&cmd);

            // Verify program name
            prop_assert_eq!(cmd.get_program(), OsStr::new("av1an"));

            // Verify input path (Requirements 10.1)
            prop_assert!(
                has_flag_with_value(&args, "-i", &input_path),
                "Command should contain -i with input path '{}', args: {:?}",
                input_path, args
            );

            // Verify output path (Requirements 10.2)
            prop_assert!(
                has_flag_with_value(&args, "-o", &output_path),
                "Command should contain -o with output path '{}', args: {:?}",
                output_path, args
            );

            // Verify encoder (Requirements 2.1, 10.3)
            prop_assert!(
                has_flag_with_value(&args, "--encoder", "svt-av1"),
                "Command should contain --encoder svt-av1, args: {:?}",
                args
            );

            // Verify pixel format (Requirements 2.2, 10.4)
            prop_assert!(
                has_flag_with_value(&args, "--pix-format", "yuv420p10le"),
                "Command should contain --pix-format yuv420p10le, args: {:?}",
                args
            );

            // Verify CRF (Requirements 2.3, 10.5)
            prop_assert!(
                has_flag_with_value(&args, "--crf", "8"),
                "Command should contain --crf 8, args: {:?}",
                args
            );

            // Verify preset (Requirements 2.4, 10.6)
            prop_assert!(
                has_flag_with_value(&args, "--preset", "3"),
                "Command should contain --preset 3, args: {:?}",
                args
            );

            // Verify SVT params (Requirements 2.5, 10.7)
            prop_assert!(
                has_flag_with_value(&args, "--svt-params", SVT_PARAMS),
                "Command should contain --svt-params with film-grain tuning, args: {:?}",
                args
            );

            // Verify target quality (Requirements 2.6, 10.8)
            prop_assert!(
                has_flag_with_value(&args, "--target-quality", "1"),
                "Command should contain --target-quality 1, args: {:?}",
                args
            );

            // Verify audio copy (Requirements 2.7, 10.9)
            prop_assert!(
                has_flag(&args, "--audio-copy"),
                "Command should contain --audio-copy, args: {:?}",
                args
            );

            // Verify workers (Requirements 10.10)
            prop_assert!(
                has_flag_with_value(&args, "--workers", &av1an_workers.to_string()),
                "Command should contain --workers {}, args: {:?}",
                av1an_workers, args
            );

            // Verify temp directory (Requirements 10.11)
            prop_assert!(
                has_flag_with_value(&args, "--temp", &temp_dir),
                "Command should contain --temp with temp dir '{}', args: {:?}",
                temp_dir, args
            );
        }
    }
}
