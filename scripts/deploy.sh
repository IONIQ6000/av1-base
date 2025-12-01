#!/bin/bash
# AV1 Super Daemon - Deploy/Update Script for Debian Trixie
# Usage: ./scripts/deploy.sh [--install-deps]
#
# This script:
# 1. Stops the running daemon service (if exists)
# 2. Pulls latest code from git
# 3. Optionally installs dependencies (--install-deps)
# 4. Builds the application in release mode
# 5. Installs binaries to /usr/local/bin
# 6. Installs/updates systemd service
# 7. Starts the daemon service

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Check if running as root for service operations
check_root() {
    if [ "$EUID" -ne 0 ]; then
        log_error "This script must be run as root for service installation"
        log_info "Usage: sudo ./scripts/deploy.sh [--install-deps]"
        exit 1
    fi
}

# Install dependencies on Debian Trixie
install_dependencies() {
    log_info "Installing dependencies for Debian Trixie..."
    
    apt-get update
    apt-get install -y \
        build-essential \
        curl \
        git \
        pkg-config \
        libssl-dev \
        meson \
        ninja-build \
        nasm \
        cmake \
        autoconf \
        automake \
        libtool \
        cython3 \
        python3-dev \
        zlib1g-dev \
        libzimg-dev \
        svt-av1
    
    # Install Rust if not present
    if ! command -v cargo &> /dev/null; then
        log_info "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
    
    # Install FFmpeg 8+ if not present or version < 8
    if ! command -v ffmpeg &> /dev/null; then
        log_warn "FFmpeg not found. Run scripts/install_ffmpeg8.sh to install FFmpeg 8+"
    else
        FFMPEG_VERSION=$(ffmpeg -version 2>&1 | head -1 | grep -oP '(?<=version )[n]?\d+' | tr -d 'n')
        if [ "$FFMPEG_VERSION" -lt 8 ]; then
            log_warn "FFmpeg version $FFMPEG_VERSION found. Version 8+ required."
            log_warn "Run scripts/install_ffmpeg8.sh to install FFmpeg 8+"
        else
            log_info "FFmpeg version $FFMPEG_VERSION found (OK)"
        fi
    fi
    
    # Install VapourSynth from source (not in Debian Trixie repos)
    if [ ! -f /usr/local/lib/libvapoursynth.so ]; then
        log_info "Building VapourSynth from source..."
        cd /tmp
        rm -rf vapoursynth
        git clone --depth 1 --branch R70 https://github.com/vapoursynth/vapoursynth.git
        cd vapoursynth
        ./autogen.sh
        ./configure --prefix=/usr/local
        make -j$(nproc)
        make install
        ldconfig
        # Set up Python path
        echo 'export PYTHONPATH=/usr/local/lib/python3.13/site-packages:$PYTHONPATH' > /etc/profile.d/vapoursynth.sh
        source /etc/profile.d/vapoursynth.sh
        log_info "VapourSynth installed successfully"
    else
        log_info "VapourSynth already installed"
    fi
    
    # Install av1an from source (requires VapourSynth)
    if ! command -v av1an &> /dev/null; then
        log_info "Building av1an from source..."
        cd /tmp
        rm -rf Av1an-latest
        git clone https://github.com/master-of-zen/Av1an.git Av1an-latest
        cd Av1an-latest
        cargo build --release
        cp target/release/av1an /usr/local/bin/
        log_info "av1an installed successfully"
    else
        log_info "av1an already installed"
    fi
    
    log_info "Dependencies installed successfully"
}

# Stop the daemon service if running
stop_service() {
    if systemctl is-active --quiet av1-super-daemon; then
        log_info "Stopping av1-super-daemon service..."
        systemctl stop av1-super-daemon
    else
        log_info "Service not running, skipping stop"
    fi
}

# Pull latest code from git
pull_latest() {
    log_info "Pulling latest code from git..."
    
    cd "$PROJECT_DIR"
    
    # Stash any local changes
    if ! git diff --quiet; then
        log_warn "Local changes detected, stashing..."
        git stash
    fi
    
    git pull --rebase origin main || git pull --rebase origin master || {
        log_error "Failed to pull from git"
        exit 1
    }
    
    log_info "Code updated successfully"
}

# Build the application
build_app() {
    log_info "Building application in release mode..."
    
    cd "$PROJECT_DIR"
    
    # Ensure cargo is in PATH
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    fi
    
    cargo build --release --workspace
    
    log_info "Build completed successfully"
}

# Install binaries
install_binaries() {
    log_info "Installing binaries to /usr/local/bin..."
    
    cp "$PROJECT_DIR/target/release/av1-super-daemon" /usr/local/bin/
    cp "$PROJECT_DIR/target/release/atop" /usr/local/bin/
    
    chmod +x /usr/local/bin/av1-super-daemon
    chmod +x /usr/local/bin/atop
    
    log_info "Binaries installed successfully"
}

# Create config directory and default config
setup_config() {
    log_info "Setting up configuration..."
    
    CONFIG_DIR="/etc/av1-super-daemon"
    mkdir -p "$CONFIG_DIR"
    
    if [ ! -f "$CONFIG_DIR/config.toml" ]; then
        log_info "Creating default configuration..."
        cat > "$CONFIG_DIR/config.toml" << 'EOF'
[cpu]
# logical_cores = 32  # Auto-detect if not set
target_cpu_utilization = 0.85

[av1an]
workers_per_job = 0      # 0 = auto-derive (8 for 32+ cores, 4 otherwise)
max_concurrent_jobs = 0  # 0 = auto-derive (1 for 24+ cores, 2 otherwise)

[encoder_safety]
disallow_hardware_encoding = true
EOF
    else
        log_info "Configuration already exists, skipping"
    fi
    
    # Create temp directory
    mkdir -p /var/lib/av1-super-daemon/chunks
    
    log_info "Configuration setup complete"
}

# Install systemd service
install_service() {
    log_info "Installing systemd service..."
    
    cat > /etc/systemd/system/av1-super-daemon.service << 'EOF'
[Unit]
Description=AV1 Super Daemon - Automated media encoding with film-grain-tuned AV1
After=network.target

[Service]
Type=simple
Environment="PYTHONPATH=/usr/local/lib/python3.13/site-packages"
ExecStart=/usr/local/bin/av1-super-daemon --config /etc/av1-super-daemon/config.toml --temp-dir /var/lib/av1-super-daemon/chunks
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/var/lib/av1-super-daemon
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable av1-super-daemon
    
    log_info "Systemd service installed and enabled"
}

# Start the service
start_service() {
    log_info "Starting av1-super-daemon service..."
    systemctl start av1-super-daemon
    
    sleep 2
    
    if systemctl is-active --quiet av1-super-daemon; then
        log_info "Service started successfully"
        log_info "Metrics available at: http://127.0.0.1:7878/metrics"
    else
        log_error "Service failed to start. Check logs with: journalctl -u av1-super-daemon"
        exit 1
    fi
}

# Show status
show_status() {
    echo ""
    log_info "=== Deployment Complete ==="
    echo ""
    echo "Service status:"
    systemctl status av1-super-daemon --no-pager || true
    echo ""
    
    # Quick test of metrics endpoint
    echo "Testing metrics endpoint..."
    sleep 1
    if curl -s http://127.0.0.1:7878/metrics > /dev/null 2>&1; then
        log_info "Metrics endpoint is responding!"
        echo "Sample metrics:"
        curl -s http://127.0.0.1:7878/metrics | head -c 200
        echo "..."
    else
        log_warn "Metrics endpoint not responding yet"
    fi
    
    echo ""
    echo "Useful commands:"
    echo "  View logs:     journalctl -u av1-super-daemon -f"
    echo "  Restart:       systemctl restart av1-super-daemon"
    echo "  Stop:          systemctl stop av1-super-daemon"
    echo "  Check metrics: curl http://127.0.0.1:7878/metrics"
    echo "  Run TUI:       atop"
    echo ""
}

# Main
main() {
    check_root
    
    # Get the directory where this script is located - set globally
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
    
    log_info "Project directory: $PROJECT_DIR"
    
    # Parse arguments
    INSTALL_DEPS=false
    for arg in "$@"; do
        case $arg in
            --install-deps)
                INSTALL_DEPS=true
                shift
                ;;
        esac
    done
    
    stop_service
    pull_latest
    
    if [ "$INSTALL_DEPS" = true ]; then
        install_dependencies
    fi
    
    build_app
    install_binaries
    setup_config
    install_service
    start_service
    show_status
}

main "$@"
