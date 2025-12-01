//! Classifier module for categorizing video source types.
//!
//! This module analyzes video files to determine if they are web-sourced
//! (streaming rips, web downloads) or disc-sourced (Blu-ray, DVD rips)
//! based on path keywords, bitrate, and resolution heuristics.

use crate::gates::ProbeResult;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Classification of video source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    /// Web-sourced content (streaming rips, web downloads).
    /// Typically lower bitrate relative to resolution.
    WebLike,
    /// Disc-sourced content (Blu-ray, DVD rips).
    /// Typically higher bitrate relative to resolution.
    DiscLike,
    /// Source type could not be determined.
    Unknown,
}

impl Default for SourceType {
    fn default() -> Self {
        Self::Unknown
    }
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::WebLike => write!(f, "web_like"),
            SourceType::DiscLike => write!(f, "disc_like"),
            SourceType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Keywords that indicate web-sourced content.
const WEB_KEYWORDS: &[&str] = &[
    "webrip", "web-rip", "webdl", "web-dl", "web.dl", "web.rip",
    "amzn", "amazon", "nf", "netflix", "hulu", "dsnp", "disney",
    "atvp", "appletv", "hmax", "hbo", "pcok", "peacock",
    "pmtp", "paramount", "stan", "it", "hdtv", "pdtv",
    "webhd", "web", "streaming",
];

/// Keywords that indicate disc-sourced content.
const DISC_KEYWORDS: &[&str] = &[
    "bluray", "blu-ray", "bdrip", "bd-rip", "brrip", "br-rip",
    "remux", "bdremux", "bd.remux", "dvdrip", "dvd-rip", "dvd",
    "uhd", "ultrahd", "4k.uhd", "hddvd", "hd-dvd",
];

/// Bitrate threshold in kbps per megapixel for web vs disc classification.
/// Content below this threshold (relative to resolution) is considered web-like.
/// Typical web content: 2-8 Mbps for 1080p (~2 MP) = 1000-4000 kbps/MP
/// Typical disc content: 20-40 Mbps for 1080p (~2 MP) = 10000-20000 kbps/MP
const BITRATE_THRESHOLD_KBPS_PER_MP: f32 = 6000.0;

/// Classifies a video source based on path keywords and probe results.
///
/// Classification logic:
/// 1. Check path for web-related keywords -> WebLike
/// 2. Check path for disc-related keywords -> DiscLike
/// 3. Analyze bitrate vs resolution ratio:
///    - Low bitrate relative to resolution -> WebLike
///    - High bitrate relative to resolution -> DiscLike
/// 4. If no determination can be made -> Unknown
pub fn classify_source(path: &Path, probe: &ProbeResult) -> SourceType {
    // Convert path to lowercase string for keyword matching
    let path_str = path.to_string_lossy().to_lowercase();

    // Check for web keywords in path
    if contains_any_keyword(&path_str, WEB_KEYWORDS) {
        return SourceType::WebLike;
    }

    // Check for disc keywords in path
    if contains_any_keyword(&path_str, DISC_KEYWORDS) {
        return SourceType::DiscLike;
    }

    // Fall back to bitrate vs resolution analysis
    classify_by_bitrate_ratio(probe)
}

/// Checks if the path string contains any of the given keywords.
fn contains_any_keyword(path_str: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| path_str.contains(kw))
}

/// Classifies source type based on bitrate to resolution ratio.
fn classify_by_bitrate_ratio(probe: &ProbeResult) -> SourceType {
    // Get the first video stream
    let video_stream = match probe.video_streams.first() {
        Some(vs) => vs,
        None => return SourceType::Unknown,
    };

    // Get bitrate - if not available, we can't classify
    let bitrate_kbps = match video_stream.bitrate_kbps {
        Some(br) if br > 0.0 => br,
        _ => return SourceType::Unknown,
    };

    // Calculate megapixels
    let width = video_stream.width as f32;
    let height = video_stream.height as f32;

    if width <= 0.0 || height <= 0.0 {
        return SourceType::Unknown;
    }

    let megapixels = (width * height) / 1_000_000.0;

    if megapixels <= 0.0 {
        return SourceType::Unknown;
    }

    // Calculate bitrate per megapixel
    let bitrate_per_mp = bitrate_kbps / megapixels;

    // Classify based on threshold
    if bitrate_per_mp < BITRATE_THRESHOLD_KBPS_PER_MP {
        SourceType::WebLike
    } else {
        SourceType::DiscLike
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::{AudioStream, FormatInfo, VideoStream};
    use proptest::prelude::*;
    use std::path::PathBuf;

    /// Helper to create a VideoStream for testing.
    fn make_video_stream(codec: &str, width: u32, height: u32, bitrate_kbps: Option<f32>) -> VideoStream {
        VideoStream {
            codec_name: codec.to_string(),
            width,
            height,
            bitrate_kbps,
        }
    }

    /// Helper to create a ProbeResult for testing.
    fn make_probe_result(
        video_streams: Vec<VideoStream>,
        audio_streams: Vec<AudioStream>,
    ) -> ProbeResult {
        ProbeResult {
            video_streams,
            audio_streams,
            format: FormatInfo {
                duration_secs: 3600.0,
                size_bytes: 5_000_000_000,
            },
        }
    }

    // Strategy for generating arbitrary file paths
    fn path_strategy() -> impl Strategy<Value = PathBuf> {
        prop::collection::vec("[a-zA-Z0-9._-]{1,20}", 1..5)
            .prop_map(|parts| PathBuf::from(parts.join("/")))
    }

    // Strategy for generating video streams with various properties
    fn video_stream_strategy() -> impl Strategy<Value = VideoStream> {
        (
            "[a-z0-9]{2,10}",           // codec name
            1u32..8000,                  // width
            1u32..4500,                  // height
            prop::option::of(1.0f32..100000.0), // bitrate_kbps
        )
            .prop_map(|(codec, width, height, bitrate)| VideoStream {
                codec_name: codec,
                width,
                height,
                bitrate_kbps: bitrate,
            })
    }

    // Strategy for generating probe results
    fn probe_result_strategy() -> impl Strategy<Value = ProbeResult> {
        prop::collection::vec(video_stream_strategy(), 0..3).prop_map(|video_streams| ProbeResult {
            video_streams,
            audio_streams: vec![],
            format: FormatInfo {
                duration_secs: 3600.0,
                size_bytes: 5_000_000_000,
            },
        })
    }

    // **Feature: av1-super-daemon, Property 18: Source Classification Consistency**
    // **Validates: Requirements 15.1, 15.4**
    //
    // *For any* path and probe result, the classifier SHALL return exactly one of
    // `WebLike`, `DiscLike`, or `Unknown` - never multiple or none.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_classification_consistency(
            path in path_strategy(),
            probe in probe_result_strategy(),
        ) {
            let result = classify_source(&path, &probe);

            // Verify the result is exactly one of the three variants
            let is_valid = matches!(
                result,
                SourceType::WebLike | SourceType::DiscLike | SourceType::Unknown
            );

            prop_assert!(
                is_valid,
                "classify_source must return exactly one of WebLike, DiscLike, or Unknown"
            );

            // Verify the result is deterministic (calling again gives same result)
            let result2 = classify_source(&path, &probe);
            prop_assert_eq!(
                result, result2,
                "classify_source must be deterministic for the same inputs"
            );
        }

        // Additional property: web keywords always result in WebLike
        #[test]
        fn prop_web_keywords_classify_as_weblike(
            base_path in "[a-zA-Z0-9]{1,10}",
            web_keyword in prop::sample::select(vec![
                "webrip", "web-dl", "webdl", "amzn", "netflix", "nf", "hulu",
                "dsnp", "disney", "atvp", "hmax", "hbo", "web"
            ]),
            probe in probe_result_strategy(),
        ) {
            let path = PathBuf::from(format!("{}/{}/video.mkv", base_path, web_keyword));
            let result = classify_source(&path, &probe);

            prop_assert_eq!(
                result,
                SourceType::WebLike,
                "Path containing web keyword '{}' should classify as WebLike, got {:?}",
                web_keyword,
                result
            );
        }

        // Additional property: disc keywords always result in DiscLike
        // Note: base_path must not contain any web keywords, since web keywords take precedence
        #[test]
        fn prop_disc_keywords_classify_as_disclike(
            base_path in "[a-zA-Z0-9]{1,10}".prop_filter(
                "base_path must not contain web keywords",
                |s| {
                    let lower = s.to_lowercase();
                    // Exclude paths that contain web keywords (which take precedence)
                    !WEB_KEYWORDS.iter().any(|kw| lower.contains(kw))
                }
            ),
            disc_keyword in prop::sample::select(vec![
                "bluray", "blu-ray", "bdrip", "remux", "bdremux", "dvdrip", "dvd", "uhd"
            ]),
            probe in probe_result_strategy(),
        ) {
            let path = PathBuf::from(format!("{}/{}/video.mkv", base_path, disc_keyword));
            let result = classify_source(&path, &probe);

            prop_assert_eq!(
                result,
                SourceType::DiscLike,
                "Path containing disc keyword '{}' should classify as DiscLike, got {:?}",
                disc_keyword,
                result
            );
        }
    }

    // Unit tests for specific scenarios
    #[test]
    fn test_classify_web_keyword_in_path() {
        let path = PathBuf::from("/media/movies/Movie.2024.WEB-DL.1080p.mkv");
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 1920, 1080, Some(5000.0))],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::WebLike);
    }

    #[test]
    fn test_classify_disc_keyword_in_path() {
        let path = PathBuf::from("/media/movies/Movie.2024.BluRay.1080p.mkv");
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 1920, 1080, Some(25000.0))],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::DiscLike);
    }

    #[test]
    fn test_classify_remux_as_disc() {
        let path = PathBuf::from("/media/movies/Movie.2024.REMUX.2160p.mkv");
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 3840, 2160, Some(50000.0))],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::DiscLike);
    }

    #[test]
    fn test_classify_by_low_bitrate() {
        // No keywords, but low bitrate relative to resolution -> WebLike
        let path = PathBuf::from("/media/movies/Movie.2024.1080p.mkv");
        let probe = make_probe_result(
            // 1080p = ~2 MP, 4000 kbps = 2000 kbps/MP (below threshold)
            vec![make_video_stream("hevc", 1920, 1080, Some(4000.0))],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::WebLike);
    }

    #[test]
    fn test_classify_by_high_bitrate() {
        // No keywords, but high bitrate relative to resolution -> DiscLike
        let path = PathBuf::from("/media/movies/Movie.2024.1080p.mkv");
        let probe = make_probe_result(
            // 1080p = ~2 MP, 25000 kbps = 12500 kbps/MP (above threshold)
            vec![make_video_stream("hevc", 1920, 1080, Some(25000.0))],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::DiscLike);
    }

    #[test]
    fn test_classify_unknown_no_video_streams() {
        let path = PathBuf::from("/media/movies/Movie.2024.mkv");
        let probe = make_probe_result(vec![], vec![]);

        assert_eq!(classify_source(&path, &probe), SourceType::Unknown);
    }

    #[test]
    fn test_classify_unknown_no_bitrate() {
        let path = PathBuf::from("/media/movies/Movie.2024.mkv");
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 1920, 1080, None)],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::Unknown);
    }

    #[test]
    fn test_classify_case_insensitive_keywords() {
        // Keywords should match case-insensitively
        let path = PathBuf::from("/media/movies/Movie.2024.WEBRIP.1080p.mkv");
        let probe = make_probe_result(
            vec![make_video_stream("hevc", 1920, 1080, Some(5000.0))],
            vec![],
        );

        assert_eq!(classify_source(&path, &probe), SourceType::WebLike);
    }

    #[test]
    fn test_source_type_display() {
        assert_eq!(format!("{}", SourceType::WebLike), "web_like");
        assert_eq!(format!("{}", SourceType::DiscLike), "disc_like");
        assert_eq!(format!("{}", SourceType::Unknown), "unknown");
    }

    #[test]
    fn test_source_type_default() {
        assert_eq!(SourceType::default(), SourceType::Unknown);
    }
}
