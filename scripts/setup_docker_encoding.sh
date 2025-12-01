#!/usr/bin/env bash
#
# Complete setup script for Docker-based AV1 encoding
# Cleans up local installs and sets up the Docker wrapper
#
# Run with: sudo ./scripts/setup_docker_encoding.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=============================================="
echo "  AV1 Super Daemon - Docker Encoding Setup"
echo "=============================================="
echo ""
echo "This will:"
echo "  1. Remove locally installed av1an and ffmpeg"
echo "  2. Pull masterofzen/av1an:master (includes ffmpeg 8.0.1)"
echo "  3. Install av1an wrapper to /usr/local/bin"
echo ""

# Check for root
if [[ $EUID -ne 0 ]]; then
    echo "Error: This script must be run as root (use sudo)" >&2
    exit 1
fi

# Check Docker is available
if ! command -v docker &>/dev/null; then
    echo "Error: Docker is not installed or not in PATH" >&2
    exit 1
fi

# Run cleanup
echo "==> Step 1: Cleaning up local installations..."
"${SCRIPT_DIR}/cleanup_local_encoders.sh"

# Install wrapper
echo ""
echo "==> Step 2: Installing av1an wrapper..."
install -m 755 "${SCRIPT_DIR}/av1an-docker" /usr/local/bin/av1an
echo "    Installed to /usr/local/bin/av1an"

# Verify
echo ""
echo "==> Step 3: Verifying installation..."
echo ""
echo "av1an version:"
/usr/local/bin/av1an --version 2>&1 | head -5 || echo "    (version check may require a video file)"
echo ""
echo "ffmpeg version (inside container):"
docker run --rm masterofzen/av1an:master ffmpeg -version 2>&1 | head -1

echo ""
echo "=============================================="
echo "  Setup Complete!"
echo "=============================================="
echo ""
echo "The daemon will now use Docker for all encoding."
echo "No code changes required - av1an calls are transparently"
echo "routed through the container."
