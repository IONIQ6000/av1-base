//! Size Gate Module
//!
//! Post-encode validation ensuring output is smaller than original by a configured ratio.

use serde::{Deserialize, Serialize};

/// Result of the size gate check
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SizeGateResult {
    /// Output size is acceptable (smaller than threshold)
    Accept,
    /// Output size exceeds threshold
    Reject {
        original_bytes: u64,
        output_bytes: u64,
        ratio: f32,
    },
}

/// Check if the output file size passes the size gate.
///
/// Returns `Reject` if `output_bytes >= original_bytes * max_ratio`,
/// otherwise returns `Accept`.
///
/// # Arguments
/// * `original_bytes` - Size of the original file in bytes
/// * `output_bytes` - Size of the encoded output file in bytes
/// * `max_ratio` - Maximum allowed ratio of output/original (e.g., 0.95 means reject if >= 95%)
pub fn check_size_gate(original_bytes: u64, output_bytes: u64, max_ratio: f32) -> SizeGateResult {
    let threshold = (original_bytes as f64 * max_ratio as f64) as u64;
    
    if output_bytes >= threshold {
        let actual_ratio = if original_bytes > 0 {
            output_bytes as f32 / original_bytes as f32
        } else {
            f32::INFINITY
        };
        SizeGateResult::Reject {
            original_bytes,
            output_bytes,
            ratio: actual_ratio,
        }
    } else {
        SizeGateResult::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // **Feature: av1-super-daemon, Property 19: Size Gate Threshold**
    // **Validates: Requirements 16.1, 16.2, 16.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        
        #[test]
        fn prop_size_gate_threshold(
            original_bytes in 1u64..=u64::MAX / 2,
            output_bytes in 0u64..=u64::MAX / 2,
            max_ratio in 0.01f32..=1.0f32,
        ) {
            let result = check_size_gate(original_bytes, output_bytes, max_ratio);
            let threshold = (original_bytes as f64 * max_ratio as f64) as u64;
            
            match result {
                SizeGateResult::Accept => {
                    prop_assert!(output_bytes < threshold,
                        "Accept returned but output_bytes ({}) >= threshold ({})",
                        output_bytes, threshold);
                }
                SizeGateResult::Reject { original_bytes: orig, output_bytes: out, ratio: _ } => {
                    prop_assert!(output_bytes >= threshold,
                        "Reject returned but output_bytes ({}) < threshold ({})",
                        output_bytes, threshold);
                    prop_assert_eq!(orig, original_bytes);
                    prop_assert_eq!(out, output_bytes);
                }
            }
        }
    }
}
