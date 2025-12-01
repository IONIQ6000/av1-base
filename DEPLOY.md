# AV1 Super Daemon - Deployment Guide

## Quick Deploy on Debian Trixie

### First-time Installation

```bash
# Clone the repository
git clone <your-repo-url> /opt/av1-super-daemon
cd /opt/av1-super-daemon

# Install dependencies and deploy
sudo ./scripts/deploy.sh --install-deps
```

### Update Existing Installation

```bash
cd /opt/av1-super-daemon
sudo ./scripts/deploy.sh
```

## Manual Installation

### 1. Install Dependencies

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    curl \
    git \
    pkg-config \
    libssl-dev \
    meson \
    ninja-build \
    nasm \
    cmake \
    libvapoursynth-dev \
    vapoursynth

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install av1an
cargo install av1an
```

### 2. Install FFmpeg 8+

Option A: Use the install script with a pre-built binary:
```bash
export FFMPEG_ARCHIVE_URL="https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz"
sudo ./scripts/install_ffmpeg8.sh
```

Option B: Build from source (if Debian Trixie has FFmpeg 8+ in repos):
```bash
sudo apt-get install -y ffmpeg
ffmpeg -version  # Verify version >= 8
```

### 3. Build and Install

```bash
cargo build --release --workspace
sudo cp target/release/av1-super-daemon /usr/local/bin/
sudo cp target/release/av1-dashboard /usr/local/bin/
```

### 4. Configure

```bash
sudo mkdir -p /etc/av1-super-daemon
sudo cp config.toml /etc/av1-super-daemon/
sudo mkdir -p /var/lib/av1-super-daemon/chunks
```

Edit `/etc/av1-super-daemon/config.toml` as needed.

### 5. Install Systemd Service

```bash
sudo tee /etc/systemd/system/av1-super-daemon.service << 'EOF'
[Unit]
Description=AV1 Super Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/av1-super-daemon --config /etc/av1-super-daemon/config.toml --temp-dir /var/lib/av1-super-daemon/chunks
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable av1-super-daemon
sudo systemctl start av1-super-daemon
```

## Usage

### Service Management

```bash
# Start/stop/restart
sudo systemctl start av1-super-daemon
sudo systemctl stop av1-super-daemon
sudo systemctl restart av1-super-daemon

# View logs
journalctl -u av1-super-daemon -f

# Check status
systemctl status av1-super-daemon
```

### Metrics & Monitoring

```bash
# Check metrics endpoint
curl http://127.0.0.1:7878/metrics

# Run TUI dashboard
av1-dashboard
```

### Configuration

Edit `/etc/av1-super-daemon/config.toml`:

```toml
[cpu]
# logical_cores = 32  # Auto-detect if not set
target_cpu_utilization = 0.85

[av1an]
workers_per_job = 0      # 0 = auto (8 for 32+ cores, 4 otherwise)
max_concurrent_jobs = 0  # 0 = auto (1 for 24+ cores, 2 otherwise)

[encoder_safety]
disallow_hardware_encoding = true
```

Environment variable overrides:
- `CPU_LOGICAL_CORES`
- `CPU_TARGET_UTILIZATION`
- `AV1AN_WORKERS_PER_JOB`
- `AV1AN_MAX_CONCURRENT_JOBS`
- `ENCODER_DISALLOW_HARDWARE_ENCODING`

## Troubleshooting

### Daemon won't start

```bash
# Check logs
journalctl -u av1-super-daemon -n 50

# Test manually
/usr/local/bin/av1-super-daemon --config /etc/av1-super-daemon/config.toml
```

### av1an not found

```bash
# Check if av1an is in PATH
which av1an

# If installed via cargo, ensure PATH includes cargo bin
export PATH="$HOME/.cargo/bin:$PATH"
```

### FFmpeg version too old

```bash
ffmpeg -version  # Check version

# Install FFmpeg 8+ using the script
export FFMPEG_ARCHIVE_URL="https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz"
sudo ./scripts/install_ffmpeg8.sh
```
