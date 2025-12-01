//! Replacer module for atomic file replacement with backup.
//!
//! This module provides functionality to safely replace original video files
//! with encoded versions, creating backups and handling errors gracefully.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Errors that can occur during file replacement.
#[derive(Debug, Error)]
pub enum ReplaceError {
    /// Failed to create backup of original file.
    #[error("Failed to create backup: {0}")]
    BackupFailed(std::io::Error),

    /// Failed to copy encoded file to original location.
    #[error("Failed to copy encoded file: {0}")]
    CopyFailed(std::io::Error),

    /// Failed to delete backup file.
    #[error("Failed to delete backup: {0}")]
    DeleteBackupFailed(std::io::Error),
}

/// Generates a backup path for the original file.
///
/// The backup path follows the format: `<name>.orig.<timestamp>`
/// where timestamp is Unix epoch seconds.
///
/// # Arguments
///
/// * `original` - Path to the original file
///
/// # Returns
///
/// A PathBuf with the backup path
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use av1_super_daemon::replace::backup_path;
///
/// let original = Path::new("/media/movies/film.mkv");
/// let backup = backup_path(original);
/// // backup will be something like "/media/movies/film.mkv.orig.1701388800"
/// ```
///
/// # Requirements
///
/// Implements Requirements 17.1: WHEN replacement begins THEN the Replacer
/// SHALL create a backup of the original file as `<name>.orig.<timestamp>`
pub fn backup_path(original: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut backup = original.as_os_str().to_owned();
    backup.push(format!(".orig.{}", timestamp));
    PathBuf::from(backup)
}


/// Atomically replaces the original file with the encoded file.
///
/// This function performs a safe file replacement with the following steps:
/// 1. Create a backup of the original file
/// 2. Copy the encoded file to the original location
/// 3. Delete the backup if `keep_original` is false
///
/// If any step fails, the function preserves both the original and encoded
/// files for manual inspection.
///
/// # Arguments
///
/// * `original_path` - Path to the original video file
/// * `encoded_path` - Path to the encoded video file
/// * `keep_original` - If true, preserve the backup file after successful replacement
///
/// # Returns
///
/// * `Ok(())` if replacement was successful
/// * `Err(ReplaceError)` if any step failed
///
/// # Requirements
///
/// Implements Requirements 17.1, 17.2, 17.3, 17.4, 17.5, 17.6:
/// - Creates backup as `<name>.orig.<timestamp>`
/// - Aborts and preserves files on backup failure
/// - Copies encoded file to original location
/// - Deletes backup if `keep_original` is false
/// - Preserves backup if `keep_original` is true
/// - Preserves temp files on any failure
pub fn atomic_replace(
    original_path: &Path,
    encoded_path: &Path,
    keep_original: bool,
) -> Result<(), ReplaceError> {
    // Step 1: Create backup of original file
    let backup = backup_path(original_path);
    
    // Try to rename first (faster, same filesystem)
    // Fall back to copy if rename fails (cross-filesystem or ZFS quirks)
    if fs::rename(original_path, &backup).is_err() {
        fs::copy(original_path, &backup)
            .map_err(ReplaceError::BackupFailed)?;
        fs::remove_file(original_path)
            .map_err(ReplaceError::BackupFailed)?;
    }

    // Step 2: Copy encoded file to original location
    if let Err(e) = fs::copy(encoded_path, original_path) {
        // Restore original from backup on failure
        let _ = fs::rename(&backup, original_path);
        return Err(ReplaceError::CopyFailed(e));
    }

    // Step 3: Delete backup if keep_original is false
    if !keep_original {
        fs::remove_file(&backup).map_err(ReplaceError::DeleteBackupFailed)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_backup_path_format() {
        let original = Path::new("/media/movies/film.mkv");
        let backup = backup_path(original);
        
        let backup_str = backup.to_string_lossy();
        assert!(backup_str.starts_with("/media/movies/film.mkv.orig."));
        
        // Extract timestamp and verify it's a valid number
        let parts: Vec<&str> = backup_str.rsplitn(2, ".orig.").collect();
        assert_eq!(parts.len(), 2);
        let timestamp: u64 = parts[0].parse().expect("Timestamp should be a number");
        assert!(timestamp > 0);
    }

    #[test]
    fn test_backup_path_preserves_extension() {
        let original = Path::new("/media/movies/film.mkv");
        let backup = backup_path(original);
        
        // The backup should contain the original extension
        let backup_str = backup.to_string_lossy();
        assert!(backup_str.contains(".mkv.orig."));
    }

    #[test]
    fn test_backup_path_handles_nested_paths() {
        let original = Path::new("/media/movies/action/film.mkv");
        let backup = backup_path(original);
        
        let backup_str = backup.to_string_lossy();
        assert!(backup_str.starts_with("/media/movies/action/film.mkv.orig."));
    }

    #[test]
    fn test_atomic_replace_success_delete_backup() {
        let temp_dir = TempDir::new().unwrap();
        
        // Create original file with content
        let original_path = temp_dir.path().join("original.mkv");
        let mut original_file = File::create(&original_path).unwrap();
        original_file.write_all(b"original content").unwrap();
        drop(original_file);
        
        // Create encoded file with different content
        let encoded_path = temp_dir.path().join("encoded.mkv");
        let mut encoded_file = File::create(&encoded_path).unwrap();
        encoded_file.write_all(b"encoded content").unwrap();
        drop(encoded_file);
        
        // Perform atomic replace with keep_original = false
        atomic_replace(&original_path, &encoded_path, false).unwrap();
        
        // Verify original location has encoded content
        let content = fs::read_to_string(&original_path).unwrap();
        assert_eq!(content, "encoded content");
        
        // Verify no backup exists (since keep_original = false)
        let entries: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().contains(".orig."))
            .collect();
        assert!(entries.is_empty(), "Backup should be deleted");
    }

    #[test]
    fn test_atomic_replace_success_keep_backup() {
        let temp_dir = TempDir::new().unwrap();
        
        // Create original file with content
        let original_path = temp_dir.path().join("original.mkv");
        let mut original_file = File::create(&original_path).unwrap();
        original_file.write_all(b"original content").unwrap();
        drop(original_file);
        
        // Create encoded file with different content
        let encoded_path = temp_dir.path().join("encoded.mkv");
        let mut encoded_file = File::create(&encoded_path).unwrap();
        encoded_file.write_all(b"encoded content").unwrap();
        drop(encoded_file);
        
        // Perform atomic replace with keep_original = true
        atomic_replace(&original_path, &encoded_path, true).unwrap();
        
        // Verify original location has encoded content
        let content = fs::read_to_string(&original_path).unwrap();
        assert_eq!(content, "encoded content");
        
        // Verify backup exists with original content
        let entries: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().contains(".orig."))
            .collect();
        assert_eq!(entries.len(), 1, "Backup should exist");
        
        let backup_content = fs::read_to_string(entries[0].path()).unwrap();
        assert_eq!(backup_content, "original content");
    }

    #[test]
    fn test_atomic_replace_preserves_on_copy_failure() {
        let temp_dir = TempDir::new().unwrap();
        
        // Create original file with content
        let original_path = temp_dir.path().join("original.mkv");
        let mut original_file = File::create(&original_path).unwrap();
        original_file.write_all(b"original content").unwrap();
        drop(original_file);
        
        // Use a non-existent encoded file to trigger copy failure
        let encoded_path = temp_dir.path().join("nonexistent.mkv");
        
        // Perform atomic replace - should fail
        let result = atomic_replace(&original_path, &encoded_path, false);
        assert!(result.is_err());
        
        // Verify original file is restored
        assert!(original_path.exists(), "Original should be restored");
        let content = fs::read_to_string(&original_path).unwrap();
        assert_eq!(content, "original content");
    }

    #[test]
    fn test_atomic_replace_backup_failure() {
        // Use a non-existent original file to trigger backup failure
        let temp_dir = TempDir::new().unwrap();
        let original_path = temp_dir.path().join("nonexistent_original.mkv");
        let encoded_path = temp_dir.path().join("encoded.mkv");
        
        // Create encoded file
        File::create(&encoded_path).unwrap();
        
        // Perform atomic replace - should fail on backup
        let result = atomic_replace(&original_path, &encoded_path, false);
        assert!(matches!(result, Err(ReplaceError::BackupFailed(_))));
    }
}
