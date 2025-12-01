//! CLI entry point for AV1 Super Daemon
//!
//! Parses command line arguments and starts the daemon.
//!
//! # Requirements
//! - 8.1: Parse config.toml for cpu, av1an, and encoder_safety sections

use av1_super_daemon::{Config, Daemon};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

/// AV1 Super Daemon - Automated media encoding with film-grain-tuned AV1
#[derive(Parser, Debug)]
#[command(name = "av1-super-daemon")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file (config.toml)
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Base directory for temporary chunk files
    #[arg(short, long, default_value = "/tmp/av1-super-daemon")]
    temp_dir: PathBuf,

    /// Skip startup checks (av1an, ffmpeg version). For testing only.
    #[arg(long, default_value = "false")]
    skip_checks: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    println!("AV1 Super Daemon starting...");
    println!("Config file: {}", args.config.display());
    println!("Temp directory: {}", args.temp_dir.display());

    // Initialize the daemon
    let daemon_result = if args.skip_checks {
        println!("WARNING: Skipping startup checks (--skip-checks enabled)");
        Config::load(&args.config)
            .map(|config| Daemon::new_without_checks(config, args.temp_dir))
            .map_err(|e| e.into())
    } else {
        Daemon::new(&args.config, args.temp_dir).await
    };

    match daemon_result {
        Ok(daemon) => {
            println!(
                "Daemon initialized with {} workers, {} max concurrent jobs",
                daemon.concurrency_plan.av1an_workers,
                daemon.concurrency_plan.max_concurrent_jobs
            );
            println!("Starting metrics server on http://127.0.0.1:7878/metrics");

            // Run the daemon with the metrics server
            if let Err(e) = daemon.run_with_server().await {
                eprintln!("Daemon error: {}", e);
                return ExitCode::FAILURE;
            }

            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Failed to initialize daemon: {}", e);
            ExitCode::FAILURE
        }
    }
}
