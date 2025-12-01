//! Skip marker module for creating skip markers and why sidecars.
//!
//! This module provides functionality to create `.av1skip` marker files
//! and optional `.why.txt` sidecar files explaining why a file was skipped.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use crate::scan::skip_marker_path;

/// Constructs the why sidecar path for a given video file.
///
/// The why sidecar is placed adjacent to the video file with `.why.txt` appended.
/// For example: `/media/movie.mkv` -> `/media/movie.mkv.why.txt`
pub fn why_sidecar_path(video_path: &Path) -> std::path::PathBuf {
    let mut sidecar_path = video_path.as_os_str().to_owned();
    sidecar_path.push(".why.txt");
    std::path::PathBuf::from(sidecar_path)
}

/// Creates an empty `.av1skip` marker file adjacent to the video file.
///
/// This marker indicates that the video should not be processed by the daemon.
/// The scanner will skip files that have this marker present.
///
/// # Arguments
///
/// * `video_path` - Path to the video file to create a skip marker for
///
/// # Returns
///
/// * `Ok(())` if the marker was created successfully
/// * `Err(io::Error)` if the marker could not be created
///
/// # Requirements
///
/// Implements Requirements 18.1: WHEN a file is skipped for any reason THEN the
/// Skip Marker Writer SHALL create a `.av1skip` file adjacent to the original
pub fn write_skip_marker(video_path: &Path) -> io::Result<()> {
    let marker_path = skip_marker_path(video_path);
    File::create(marker_path)?;
    Ok(())
}

/// Creates a `.why.txt` sidecar file with the skip reason.
///
/// This sidecar explains why a file was skipped, useful for debugging
/// and understanding the daemon's decisions.
///
/// # Arguments
///
/// * `video_path` - Path to the video file to create a why sidecar for
/// * `reason` - The reason the file was skipped
/// * `enabled` - Whether to actually write the sidecar (from config)
///
/// # Returns
///
/// * `Ok(())` if the sidecar was created successfully or if disabled
/// * `Err(io::Error)` if the sidecar could not be created
///
/// # Requirements
///
/// Implements Requirements 18.2: WHEN `write_why_sidecars` is enabled THEN the
/// Skip Marker Writer SHALL create a `.why.txt` file with the skip reason
pub fn write_why_sidecar(video_path: &Path, reason: &str, enabled: bool) -> io::Result<()> {
    if !enabled {
        return Ok(());
    }

    let sidecar_path = why_sidecar_path(video_path);
    let mut file = File::create(sidecar_path)?;
    writeln!(file, "{}", reason)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_why_sidecar_path() {
        let video = std::path::Path::new("/media/movies/film.mkv");
        let sidecar = why_sidecar_path(video);
        assert_eq!(
            sidecar,
            std::path::PathBuf::from("/media/movies/film.mkv.why.txt")
        );
    }

    #[test]
    fn test_write_skip_marker_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let video_path = temp_dir.path().join("test_video.mkv");

        // Create a dummy video file
        File::create(&video_path).unwrap();

        // Write skip marker
        write_skip_marker(&video_path).unwrap();

        // Verify marker exists
        let marker_path = skip_marker_path(&video_path);
        assert!(marker_path.exists(), "Skip marker should exist");

        // Verify marker is empty
        let content = fs::read_to_string(&marker_path).unwrap();
        assert!(content.is_empty(), "Skip marker should be empty");
    }

    #[test]
    fn test_write_why_sidecar_when_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let video_path = temp_dir.path().join("test_video.mkv");

        // Create a dummy video file
        File::create(&video_path).unwrap();

        let reason = "already AV1";

        // Write why sidecar with enabled=true
        write_why_sidecar(&video_path, reason, true).unwrap();

        // Verify sidecar exists
        let sidecar_path = why_sidecar_path(&video_path);
        assert!(sidecar_path.exists(), "Why sidecar should exist");

        // Verify sidecar contains the reason
        let content = fs::read_to_string(&sidecar_path).unwrap();
        assert!(
            content.contains(reason),
            "Why sidecar should contain the reason"
        );
    }

    #[test]
    fn test_write_why_sidecar_when_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let video_path = temp_dir.path().join("test_video.mkv");

        // Create a dummy video file
        File::create(&video_path).unwrap();

        let reason = "already AV1";

        // Write why sidecar with enabled=false
        write_why_sidecar(&video_path, reason, false).unwrap();

        // Verify sidecar does NOT exist
        let sidecar_path = why_sidecar_path(&video_path);
        assert!(
            !sidecar_path.exists(),
            "Why sidecar should NOT exist when disabled"
        );
    }

    #[test]
    fn test_write_both_marker_and_sidecar() {
        let temp_dir = TempDir::new().unwrap();
        let video_path = temp_dir.path().join("test_video.mkv");

        // Create a dummy video file
        File::create(&video_path).unwrap();

        let reason = "below minimum size";

        // Write both marker and sidecar
        write_skip_marker(&video_path).unwrap();
        write_why_sidecar(&video_path, reason, true).unwrap();

        // Verify both exist
        let marker_path = skip_marker_path(&video_path);
        let sidecar_path = why_sidecar_path(&video_path);

        assert!(marker_path.exists(), "Skip marker should exist");
        assert!(sidecar_path.exists(), "Why sidecar should exist");

        // Verify sidecar content
        let content = fs::read_to_string(&sidecar_path).unwrap();
        assert!(content.contains(reason));
    }
}
