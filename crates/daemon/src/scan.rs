//! Scanner module for discovering video files in library directories.
//!
//! This module provides functionality to recursively scan configured library roots
//! for video files, filtering by extension and skip markers.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Video file extensions supported by the scanner (case-insensitive matching).
pub const VIDEO_EXTENSIONS: &[&str] = &[".mkv", ".mp4", ".avi", ".mov", ".m4v", ".ts", ".m2ts"];

/// A candidate video file discovered during library scanning.
#[derive(Debug, Clone)]
pub struct ScanCandidate {
    /// Full path to the video file.
    pub path: PathBuf,
    /// File size in bytes at discovery time.
    pub size_bytes: u64,
    /// Last modified time of the file.
    pub modified_time: SystemTime,
}

/// Constructs the skip marker path for a given video file.
///
/// The skip marker is placed adjacent to the video file with `.av1skip` appended.
/// For example: `/media/movie.mkv` -> `/media/movie.mkv.av1skip`
pub fn skip_marker_path(video_path: &Path) -> PathBuf {
    let mut marker_path = video_path.as_os_str().to_owned();
    marker_path.push(".av1skip");
    PathBuf::from(marker_path)
}

/// Checks if a skip marker exists for the given video file.
pub fn has_skip_marker(video_path: &Path) -> bool {
    skip_marker_path(video_path).exists()
}

/// Checks if a file has a video extension (case-insensitive).
pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext_lower = format!(".{}", ext.to_lowercase());
            VIDEO_EXTENSIONS.contains(&ext_lower.as_str())
        })
        .unwrap_or(false)
}

/// Scans the given library roots for video files.
///
/// This function:
/// - Recursively walks each library root directory
/// - Skips hidden directories (names starting with `.`)
/// - Filters files by video extensions (case-insensitive)
/// - Excludes files with existing `.av1skip` markers
/// - Captures file size and modified time for stability checking
pub fn scan_libraries(roots: &[PathBuf]) -> Vec<ScanCandidate> {
    use walkdir::WalkDir;

    let mut candidates = Vec::new();

    for root in roots {
        if !root.exists() {
            continue;
        }

        let walker = WalkDir::new(root).into_iter().filter_entry(|entry| {
            // Skip hidden directories (but allow hidden files to be filtered later)
            if entry.file_type().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    // Allow the root directory even if it starts with '.'
                    if name.starts_with('.') && entry.depth() > 0 {
                        return false;
                    }
                }
            }
            true
        });

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();

            // Only process files
            if !entry.file_type().is_file() {
                continue;
            }

            // Check if it's a video file
            if !is_video_file(path) {
                continue;
            }

            // Skip files with existing skip markers
            if has_skip_marker(path) {
                continue;
            }

            // Get file metadata
            if let Ok(metadata) = entry.metadata() {
                let size_bytes = metadata.len();
                let modified_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

                candidates.push(ScanCandidate {
                    path: path.to_path_buf(),
                    size_bytes,
                    modified_time,
                });
            }
        }
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fs::{self, File};
    use tempfile::TempDir;

    #[test]
    fn test_video_extensions_defined() {
        assert!(VIDEO_EXTENSIONS.contains(&".mkv"));
        assert!(VIDEO_EXTENSIONS.contains(&".mp4"));
        assert!(VIDEO_EXTENSIONS.contains(&".avi"));
        assert!(VIDEO_EXTENSIONS.contains(&".mov"));
        assert!(VIDEO_EXTENSIONS.contains(&".m4v"));
        assert!(VIDEO_EXTENSIONS.contains(&".ts"));
        assert!(VIDEO_EXTENSIONS.contains(&".m2ts"));
        assert_eq!(VIDEO_EXTENSIONS.len(), 7);
    }

    #[test]
    fn test_is_video_file() {
        assert!(is_video_file(Path::new("/media/movie.mkv")));
        assert!(is_video_file(Path::new("/media/movie.MKV"))); // case-insensitive
        assert!(is_video_file(Path::new("/media/movie.Mp4")));
        assert!(is_video_file(Path::new("/media/movie.m2ts")));
        assert!(!is_video_file(Path::new("/media/movie.txt")));
        assert!(!is_video_file(Path::new("/media/movie.jpg")));
        assert!(!is_video_file(Path::new("/media/movie"))); // no extension
    }

    #[test]
    fn test_skip_marker_path() {
        let video = Path::new("/media/movies/film.mkv");
        let marker = skip_marker_path(video);
        assert_eq!(marker, PathBuf::from("/media/movies/film.mkv.av1skip"));
    }

    #[test]
    fn test_skip_marker_path_with_dots_in_name() {
        let video = Path::new("/media/movies/film.2024.mkv");
        let marker = skip_marker_path(video);
        assert_eq!(marker, PathBuf::from("/media/movies/film.2024.mkv.av1skip"));
    }

    // **Feature: av1-super-daemon, Property 9: Scanner Video Extension Filtering**
    // **Validates: Requirements 11.3**
    //
    // *For any* file path, the scanner SHALL include it as a candidate if and only if
    // its extension (case-insensitive) is one of: `.mkv`, `.mp4`, `.avi`, `.mov`, `.m4v`, `.ts`, `.m2ts`.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_video_extension_filtering(
            basename in "[a-zA-Z0-9_-]{1,20}",
            ext in prop_oneof![
                // Video extensions (should pass)
                Just("mkv"), Just("MKV"), Just("Mkv"),
                Just("mp4"), Just("MP4"), Just("Mp4"),
                Just("avi"), Just("AVI"), Just("Avi"),
                Just("mov"), Just("MOV"), Just("Mov"),
                Just("m4v"), Just("M4V"), Just("M4v"),
                Just("ts"), Just("TS"), Just("Ts"),
                Just("m2ts"), Just("M2TS"), Just("M2Ts"),
                // Non-video extensions (should fail)
                Just("txt"), Just("jpg"), Just("png"), Just("pdf"),
                Just("doc"), Just("exe"), Just("zip"), Just("srt"),
            ],
        ) {
            let path = PathBuf::from(format!("/media/{}.{}", basename, ext));
            let is_video = is_video_file(&path);

            // Determine if extension is a video extension (case-insensitive)
            let ext_lower = ext.to_lowercase();
            let expected_video = matches!(
                ext_lower.as_str(),
                "mkv" | "mp4" | "avi" | "mov" | "m4v" | "ts" | "m2ts"
            );

            prop_assert_eq!(
                is_video, expected_video,
                "Extension '{}' should {} be recognized as video, but is_video_file returned {}",
                ext, if expected_video { "" } else { "not" }, is_video
            );
        }
    }

    // **Feature: av1-super-daemon, Property 10: Scanner Hidden Directory Exclusion**
    // **Validates: Requirements 11.2**
    //
    // *For any* directory tree, the scanner SHALL never return files that are descendants
    // of directories whose names start with `.` (hidden directories).
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        #[test]
        fn prop_hidden_directory_exclusion(
            visible_dir in "[a-zA-Z0-9]{1,10}",
            hidden_dir in "\\.[a-zA-Z0-9]{1,10}",
            filename in "[a-zA-Z0-9]{1,10}",
        ) {
            let temp_dir = TempDir::new().unwrap();
            let root = temp_dir.path();

            // Create a visible directory with a video file
            let visible_path = root.join(&visible_dir);
            fs::create_dir_all(&visible_path).unwrap();
            let visible_video = visible_path.join(format!("{}.mkv", filename));
            File::create(&visible_video).unwrap();

            // Create a hidden directory with a video file
            let hidden_path = root.join(&hidden_dir);
            fs::create_dir_all(&hidden_path).unwrap();
            let hidden_video = hidden_path.join(format!("{}.mkv", filename));
            File::create(&hidden_video).unwrap();

            // Scan the root
            let candidates = scan_libraries(&[root.to_path_buf()]);

            // Visible video should be found
            let found_visible = candidates.iter().any(|c| c.path == visible_video);
            prop_assert!(
                found_visible,
                "Video in visible directory should be found: {:?}",
                visible_video
            );

            // Hidden video should NOT be found
            let found_hidden = candidates.iter().any(|c| c.path == hidden_video);
            prop_assert!(
                !found_hidden,
                "Video in hidden directory should NOT be found: {:?}",
                hidden_video
            );
        }
    }

    // **Feature: av1-super-daemon, Property 11: Scanner Skip Marker Exclusion**
    // **Validates: Requirements 11.4, 18.3, 18.4**
    //
    // *For any* video file path, if a corresponding `.av1skip` marker file exists
    // (same directory, `<filename>.av1skip`), the scanner SHALL exclude that file from candidates.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        #[test]
        fn prop_skip_marker_exclusion(
            filename_with_marker in "[a-zA-Z0-9]{1,10}",
            filename_without_marker in "[a-zA-Z0-9]{1,10}",
        ) {
            // Ensure filenames are different
            prop_assume!(filename_with_marker != filename_without_marker);

            let temp_dir = TempDir::new().unwrap();
            let root = temp_dir.path();

            // Create video file WITH skip marker
            let video_with_marker = root.join(format!("{}.mkv", filename_with_marker));
            File::create(&video_with_marker).unwrap();
            let marker_path = skip_marker_path(&video_with_marker);
            File::create(&marker_path).unwrap();

            // Create video file WITHOUT skip marker
            let video_without_marker = root.join(format!("{}.mkv", filename_without_marker));
            File::create(&video_without_marker).unwrap();

            // Scan the root
            let candidates = scan_libraries(&[root.to_path_buf()]);

            // Video with marker should NOT be found
            let found_with_marker = candidates.iter().any(|c| c.path == video_with_marker);
            prop_assert!(
                !found_with_marker,
                "Video with skip marker should NOT be found: {:?}",
                video_with_marker
            );

            // Video without marker should be found
            let found_without_marker = candidates.iter().any(|c| c.path == video_without_marker);
            prop_assert!(
                found_without_marker,
                "Video without skip marker should be found: {:?}",
                video_without_marker
            );
        }
    }

    // **Feature: av1-super-daemon, Property 20: Skip Marker Path Construction**
    // **Validates: Requirements 18.4**
    //
    // *For any* video file path `/dir/file.ext`, the skip marker path SHALL be `/dir/file.ext.av1skip`.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_skip_marker_path_construction(
            dir in "[a-zA-Z0-9/_-]{1,30}",
            filename in "[a-zA-Z0-9._-]{1,20}",
            ext in prop_oneof![Just("mkv"), Just("mp4"), Just("avi"), Just("mov")],
        ) {
            let video_path = PathBuf::from(format!("/{}/{}.{}", dir, filename, ext));
            let marker = skip_marker_path(&video_path);

            // Marker should be video path + ".av1skip"
            let expected = PathBuf::from(format!("/{}/{}.{}.av1skip", dir, filename, ext));
            prop_assert_eq!(
                marker.clone(), expected.clone(),
                "Skip marker for {:?} should be {:?}, got {:?}",
                video_path, expected, marker
            );

            // Marker should be in the same directory as the video
            prop_assert_eq!(
                marker.parent(), video_path.parent(),
                "Skip marker should be in same directory as video"
            );

            // Marker filename should end with .av1skip
            let marker_name = marker.file_name().unwrap().to_str().unwrap();
            prop_assert!(
                marker_name.ends_with(".av1skip"),
                "Skip marker filename should end with .av1skip: {}",
                marker_name
            );
        }
    }
}
