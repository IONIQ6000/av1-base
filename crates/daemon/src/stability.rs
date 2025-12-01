//! Stability checking module for verifying files are not being written to.
//!
//! Before processing a file, we verify it's stable (not being written to)
//! by checking if its size remains unchanged over a configurable time window.

use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

/// Result of a stability check on a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StabilityResult {
    /// File size remained unchanged during the stability window.
    Stable,
    /// File size changed during the stability window.
    Unstable {
        /// Size when first checked.
        initial_size: u64,
        /// Size after waiting.
        current_size: u64,
    },
}

/// Check if a file is stable by comparing its size before and after a wait period.
///
/// # Arguments
/// * `path` - Path to the file to check
/// * `initial_size` - The file size when first discovered
/// * `wait_secs` - How long to wait before re-checking (default 10 seconds)
///
/// # Returns
/// * `Ok(StabilityResult::Stable)` if the file size is unchanged
/// * `Ok(StabilityResult::Unstable { .. })` if the file size changed
/// * `Err` if the file cannot be read
pub async fn check_stability(
    path: &Path,
    initial_size: u64,
    wait_secs: u64,
) -> Result<StabilityResult, std::io::Error> {
    // Wait for the configured duration
    sleep(Duration::from_secs(wait_secs)).await;

    // Get current file size
    let metadata = tokio::fs::metadata(path).await?;
    let current_size = metadata.len();

    // Compare sizes
    Ok(compare_sizes(initial_size, current_size))
}

/// Compare two file sizes and return the appropriate StabilityResult.
///
/// This is a pure function extracted for property testing.
#[inline]
pub fn compare_sizes(initial_size: u64, current_size: u64) -> StabilityResult {
    if initial_size == current_size {
        StabilityResult::Stable
    } else {
        StabilityResult::Unstable {
            initial_size,
            current_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // **Feature: av1-super-daemon, Property 12: Stability Check Size Comparison**
    // **Validates: Requirements 12.2, 12.3, 12.4**
    proptest! {
        #[test]
        fn prop_stability_size_comparison(initial_size: u64, current_size: u64) {
            let result = compare_sizes(initial_size, current_size);

            if initial_size == current_size {
                // Requirements 12.4: When the file size remains unchanged, mark as stable
                prop_assert_eq!(result, StabilityResult::Stable);
            } else {
                // Requirements 12.3: When the file size has changed, mark as unstable
                match result {
                    StabilityResult::Unstable { initial_size: i, current_size: c } => {
                        prop_assert_eq!(i, initial_size);
                        prop_assert_eq!(c, current_size);
                    }
                    StabilityResult::Stable => {
                        prop_assert!(false, "Expected Unstable when sizes differ");
                    }
                }
            }
        }
    }

    #[test]
    fn test_compare_sizes_stable() {
        let result = compare_sizes(1000, 1000);
        assert_eq!(result, StabilityResult::Stable);
    }

    #[test]
    fn test_compare_sizes_unstable_larger() {
        let result = compare_sizes(1000, 2000);
        assert_eq!(
            result,
            StabilityResult::Unstable {
                initial_size: 1000,
                current_size: 2000
            }
        );
    }

    #[test]
    fn test_compare_sizes_unstable_smaller() {
        let result = compare_sizes(2000, 1000);
        assert_eq!(
            result,
            StabilityResult::Unstable {
                initial_size: 2000,
                current_size: 1000
            }
        );
    }
}
