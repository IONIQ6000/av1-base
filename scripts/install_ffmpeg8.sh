#!/usr/bin/env bash
#
# FFmpeg 8+ Installer Script for Debian-based hosts
# 
# This script installs FFmpeg 8+ from a pre-built archive.
# It requires root privileges and the FFMPEG_ARCHIVE_URL environment variable.
#
# Requirements validated:
#   9.1 - Check for root privileges
#   9.2 - Remove existing distro FFmpeg
#   9.3 - Download from FFMPEG_ARCHIVE_URL
#   9.4 - Exit if FFMPEG_ARCHIVE_URL not set
#   9.5 - Install to /usr/local/bin
#   9.6 - Display installed version

set -euo pipefail

# Requirement 9.1: Check for root privileges
if [[ $EUID -ne 0 ]]; then
    echo "Error: This script must be run as root (use sudo)" >&2
    exit 1
fi

# Requirement 9.4: Check FFMPEG_ARCHIVE_URL is set
if [[ -z "${FFMPEG_ARCHIVE_URL:-}" ]]; then
    echo "Error: FFMPEG_ARCHIVE_URL environment variable is not set" >&2
    echo "Please set it to the URL of the FFmpeg 8+ archive (e.g., .tar.xz)" >&2
    exit 1
fi

echo "==> FFmpeg 8+ Installer"
echo "    Archive URL: ${FFMPEG_ARCHIVE_URL}"

# Requirement 9.2: Remove existing distro-installed FFmpeg
echo "==> Removing existing distro FFmpeg packages..."
apt-get remove -y ffmpeg libavcodec-dev libavformat-dev libavutil-dev \
    libswscale-dev libswresample-dev libavfilter-dev 2>/dev/null || true
apt-get autoremove -y 2>/dev/null || true

# Create temporary directory for download and extraction
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "${TEMP_DIR}"' EXIT

# Requirement 9.3: Download from FFMPEG_ARCHIVE_URL
echo "==> Downloading FFmpeg archive..."
ARCHIVE_FILE="${TEMP_DIR}/ffmpeg.tar.xz"
curl -fSL "${FFMPEG_ARCHIVE_URL}" -o "${ARCHIVE_FILE}"

# Extract the archive
echo "==> Extracting archive..."
tar -xf "${ARCHIVE_FILE}" -C "${TEMP_DIR}"

# Find the extracted directory (handles various naming conventions)
EXTRACTED_DIR=$(find "${TEMP_DIR}" -maxdepth 1 -type d -name "ffmpeg*" | head -n 1)

if [[ -z "${EXTRACTED_DIR}" ]]; then
    # Try to find any directory that contains ffmpeg binary
    EXTRACTED_DIR=$(find "${TEMP_DIR}" -maxdepth 2 -type f -name "ffmpeg" -exec dirname {} \; | head -n 1)
fi

if [[ -z "${EXTRACTED_DIR}" ]]; then
    echo "Error: Could not find ffmpeg in extracted archive" >&2
    exit 1
fi

# Requirement 9.5: Install ffmpeg and ffprobe to /usr/local/bin
echo "==> Installing ffmpeg and ffprobe to /usr/local/bin..."

# Handle different archive structures
if [[ -f "${EXTRACTED_DIR}/ffmpeg" ]]; then
    install -m 755 "${EXTRACTED_DIR}/ffmpeg" /usr/local/bin/ffmpeg
    install -m 755 "${EXTRACTED_DIR}/ffprobe" /usr/local/bin/ffprobe
elif [[ -f "${EXTRACTED_DIR}/bin/ffmpeg" ]]; then
    install -m 755 "${EXTRACTED_DIR}/bin/ffmpeg" /usr/local/bin/ffmpeg
    install -m 755 "${EXTRACTED_DIR}/bin/ffprobe" /usr/local/bin/ffprobe
else
    # Search for binaries
    FFMPEG_BIN=$(find "${TEMP_DIR}" -type f -name "ffmpeg" -executable | head -n 1)
    FFPROBE_BIN=$(find "${TEMP_DIR}" -type f -name "ffprobe" -executable | head -n 1)
    
    if [[ -z "${FFMPEG_BIN}" ]] || [[ -z "${FFPROBE_BIN}" ]]; then
        echo "Error: Could not find ffmpeg/ffprobe binaries in archive" >&2
        exit 1
    fi
    
    install -m 755 "${FFMPEG_BIN}" /usr/local/bin/ffmpeg
    install -m 755 "${FFPROBE_BIN}" /usr/local/bin/ffprobe
fi

# Requirement 9.6: Display installed version
echo "==> Installation complete!"
echo ""
echo "Installed FFmpeg version:"
/usr/local/bin/ffmpeg -version | head -n 1
echo ""
echo "Installed FFprobe version:"
/usr/local/bin/ffprobe -version | head -n 1
