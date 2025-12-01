#!/usr/bin/env bash
#
# Cleanup script for locally installed av1an and ffmpeg
# Removes system-installed binaries in preparation for Docker-based encoding
#
# Run with: sudo ./scripts/cleanup_local_encoders.sh

set -euo pipefail

echo "==> AV1 Encoder Cleanup Script"
echo "    Preparing system for Docker-based encoding with masterofzen/av1an:master"
echo ""

# Check for root privileges
if [[ $EUID -ne 0 ]]; then
    echo "Error: This script must be run as root (use sudo)" >&2
    exit 1
fi

# Remove ffmpeg from /usr/local/bin (installed by install_ffmpeg8.sh)
echo "==> Removing /usr/local/bin ffmpeg binaries..."
rm -f /usr/local/bin/ffmpeg /usr/local/bin/ffprobe
echo "    Done."

# Remove distro-installed ffmpeg packages
echo "==> Removing distro ffmpeg packages..."
apt-get remove -y ffmpeg libavcodec-dev libavformat-dev libavutil-dev \
    libswscale-dev libswresample-dev libavfilter-dev 2>/dev/null || true
apt-get autoremove -y 2>/dev/null || true
echo "    Done."

# Remove av1an if installed via cargo
echo "==> Checking for cargo-installed av1an..."
if command -v av1an &>/dev/null; then
    AV1AN_PATH=$(which av1an)
    echo "    Found av1an at: ${AV1AN_PATH}"
    
    # If it's in cargo bin, remove it
    if [[ "${AV1AN_PATH}" == *".cargo/bin/av1an"* ]]; then
        echo "    Removing cargo-installed av1an..."
        rm -f "${AV1AN_PATH}"
    else
        echo "    Removing av1an from ${AV1AN_PATH}..."
        rm -f "${AV1AN_PATH}"
    fi
fi
echo "    Done."

# Remove av1an if installed via apt/snap
echo "==> Removing distro av1an packages..."
apt-get remove -y av1an 2>/dev/null || true
snap remove av1an 2>/dev/null || true
echo "    Done."

# Remove SVT-AV1 encoder if installed locally
echo "==> Removing local SVT-AV1..."
apt-get remove -y svt-av1 libsvtav1enc-dev libsvtav1dec-dev 2>/dev/null || true
rm -f /usr/local/bin/SvtAv1EncApp /usr/local/bin/SvtAv1DecApp 2>/dev/null || true
echo "    Done."

# Clean up any remaining encoder binaries in common locations
echo "==> Cleaning up common binary locations..."
rm -f /usr/bin/av1an /usr/bin/ffmpeg /usr/bin/ffprobe 2>/dev/null || true
echo "    Done."

# Pull the Docker image
echo ""
echo "==> Pulling Docker image masterofzen/av1an:master..."
docker pull masterofzen/av1an:master

echo ""
echo "==> Cleanup complete!"
echo ""
echo "Docker image info:"
docker images masterofzen/av1an:master --format "    Repository: {{.Repository}}\n    Tag: {{.Tag}}\n    Size: {{.Size}}"
echo ""
echo "Next steps:"
echo "  1. Install the av1an wrapper: sudo cp scripts/av1an-docker /usr/local/bin/av1an"
echo "  2. Make it executable: sudo chmod +x /usr/local/bin/av1an"
echo "  3. Test: av1an --version"
