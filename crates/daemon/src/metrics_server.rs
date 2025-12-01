//! Metrics HTTP Server for AV1 Super Daemon
//!
//! Exposes metrics via HTTP endpoint for TUI dashboard and monitoring tools.

use axum::{extract::State, routing::get, Json, Router};
use std::net::SocketAddr;
use thiserror::Error;

use crate::metrics::{MetricsSnapshot, SharedMetrics};

/// Errors that can occur when running the metrics server
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Failed to bind to address: {0}")]
    BindError(#[from] std::io::Error),
}

/// Handler for GET /metrics endpoint
/// Returns the current MetricsSnapshot as JSON
async fn get_metrics(State(metrics): State<SharedMetrics>) -> Json<MetricsSnapshot> {
    let snapshot = metrics.read().await.clone();
    Json(snapshot)
}

/// Creates the axum Router with metrics endpoint
pub fn create_metrics_router(metrics: SharedMetrics) -> Router {
    Router::new()
        .route("/metrics", get(get_metrics))
        .with_state(metrics)
}

/// Runs the metrics HTTP server on 127.0.0.1:7878
///
/// # Arguments
/// * `metrics` - Shared metrics state to serve
///
/// # Returns
/// * `Ok(())` if server shuts down gracefully
/// * `Err(ServerError)` if server fails to start
pub async fn run_metrics_server(metrics: SharedMetrics) -> Result<(), ServerError> {
    let app = create_metrics_router(metrics);
    let addr = SocketAddr::from(([127, 0, 0, 1], 7878));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(|e| ServerError::BindError(e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{new_shared_metrics, JobMetrics, SystemMetrics};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_get_metrics_returns_json() {
        // Create shared metrics with some test data
        let metrics = new_shared_metrics();
        {
            let mut snapshot = metrics.write().await;
            snapshot.timestamp_unix_ms = 1701388800000;
            snapshot.queue_len = 5;
            snapshot.running_jobs = 1;
            snapshot.completed_jobs = 42;
            snapshot.failed_jobs = 2;
            snapshot.total_bytes_encoded = 107374182400;
            snapshot.system = SystemMetrics {
                cpu_usage_percent: 85.2,
                mem_usage_percent: 42.1,
                load_avg_1: 27.5,
                load_avg_5: 26.8,
                load_avg_15: 25.2,
            };
            snapshot.jobs.push(JobMetrics {
                id: "job-001".to_string(),
                input_path: "/media/video.mkv".to_string(),
                stage: "encoding".to_string(),
                progress: 0.45,
                fps: 12.5,
                bitrate_kbps: 8500.0,
                crf: 8,
                encoder: "svt-av1".to_string(),
                workers: 8,
                est_remaining_secs: 3600.0,
                frames_encoded: 54000,
                total_frames: 120000,
                size_in_bytes_before: 5368709120,
                size_in_bytes_after: 2147483648,
                vmaf: None,
                psnr: None,
                ssim: None,
            });
        }

        let app = create_metrics_router(metrics.clone());

        // Make request to /metrics
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Verify status code
        assert_eq!(response.status(), StatusCode::OK);

        // Verify content type is JSON
        let content_type = response
            .headers()
            .get("content-type")
            .expect("should have content-type header");
        assert!(content_type
            .to_str()
            .unwrap()
            .contains("application/json"));

        // Parse response body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let snapshot: MetricsSnapshot =
            serde_json::from_slice(&body).expect("should deserialize to MetricsSnapshot");

        // Verify data matches what we set
        assert_eq!(snapshot.timestamp_unix_ms, 1701388800000);
        assert_eq!(snapshot.queue_len, 5);
        assert_eq!(snapshot.running_jobs, 1);
        assert_eq!(snapshot.completed_jobs, 42);
        assert_eq!(snapshot.failed_jobs, 2);
        assert_eq!(snapshot.total_bytes_encoded, 107374182400);
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.jobs[0].id, "job-001");
    }

    #[tokio::test]
    async fn test_get_metrics_empty_snapshot() {
        // Create shared metrics with default (empty) data
        let metrics = new_shared_metrics();

        let app = create_metrics_router(metrics);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let snapshot: MetricsSnapshot = serde_json::from_slice(&body).unwrap();

        // Verify default values
        assert_eq!(snapshot.timestamp_unix_ms, 0);
        assert_eq!(snapshot.jobs.len(), 0);
        assert_eq!(snapshot.queue_len, 0);
        assert_eq!(snapshot.running_jobs, 0);
    }

    #[tokio::test]
    async fn test_metrics_json_format_matches_spec() {
        let metrics = new_shared_metrics();
        {
            let mut snapshot = metrics.write().await;
            snapshot.timestamp_unix_ms = 1701388800000;
            snapshot.system = SystemMetrics {
                cpu_usage_percent: 85.2,
                mem_usage_percent: 42.1,
                load_avg_1: 27.5,
                load_avg_5: 26.8,
                load_avg_15: 25.2,
            };
        }

        let app = create_metrics_router(metrics);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_str = String::from_utf8(body.to_vec()).unwrap();

        // Verify JSON contains expected field names as per design spec
        assert!(json_str.contains("timestamp_unix_ms"));
        assert!(json_str.contains("jobs"));
        assert!(json_str.contains("system"));
        assert!(json_str.contains("cpu_usage_percent"));
        assert!(json_str.contains("mem_usage_percent"));
        assert!(json_str.contains("load_avg_1"));
        assert!(json_str.contains("load_avg_5"));
        assert!(json_str.contains("load_avg_15"));
        assert!(json_str.contains("queue_len"));
        assert!(json_str.contains("running_jobs"));
        assert!(json_str.contains("completed_jobs"));
        assert!(json_str.contains("failed_jobs"));
        assert!(json_str.contains("total_bytes_encoded"));
    }
}
