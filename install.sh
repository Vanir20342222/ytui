#!/usr/bin/env bash
# ==============================================================================
# ytui - One-Command Installer Script
# ==============================================================================
# Installs ytui into ~/.local/share/ytui and links the executable to ~/.local/bin/ytui
# ==============================================================================

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${CYAN}"
echo "  ██╗   ██╗████████╗██╗   ██╗██╗"
echo "  ╚██╗ ██╔╝╚══██╔══╝██║   ██║██║"
echo "  ╚████╔╝    ██║   ██║   ██║██║"
echo "   ╚██╔╝     ██║   ██║   ██║██║"
echo "    ██║      ██║   ╚██████╔╝██║"
echo "    ╚═╝      ╚═╝    ╚═════╝ ╚═╝"
echo -e "${NC}"
echo -e "${CYAN}Terminal YouTube & Audio Downloader - Installer${NC}\n"

# 1. Dependency checks
echo -e "${CYAN}[1/4] Checking system dependencies...${NC}"

if ! command -v python3 &>/dev/null; then
    echo -e "${RED}Error: python3 is not installed. Please install Python 3.11+ first.${NC}"
    exit 1
fi

PYTHON_VERSION=$(python3 -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')
echo -e "      Found Python ${GREEN}${PYTHON_VERSION}${NC}"

if ! command -v ffmpeg &>/dev/null; then
    echo -e "${YELLOW}Warning: ffmpeg was not found in your PATH. Audio conversion & post-processing may fail.${NC}"
    echo -e "${YELLOW}Install ffmpeg via your package manager (e.g. 'sudo apt install ffmpeg').${NC}"
fi

# 2. Determine installation target
INSTALL_DIR="$HOME/.local/share/ytui"
BIN_DIR="$HOME/.local/bin"

mkdir -p "$BIN_DIR"

# Check if installing from local repo or cloning
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ -f "$SCRIPT_DIR/pyproject.toml" ]; then
    echo -e "${CYAN}[2/4] Installing from local directory (${SCRIPT_DIR})...${NC}"
    SOURCE_DIR="$SCRIPT_DIR"
else
    echo -e "${CYAN}[2/4] Setting up isolated directory in ${INSTALL_DIR}...${NC}"
    mkdir -p "$INSTALL_DIR"
    SOURCE_DIR="$INSTALL_DIR/repo"
    if [ ! -d "$SOURCE_DIR" ]; then
        echo -e "      Cloning repository..."
        git clone https://github.com/vanir/ytui.git "$SOURCE_DIR"
    else
        echo -e "      Updating existing repository..."
        git -C "$SOURCE_DIR" pull
    fi
fi

# 3. Setup Virtual Environment
VENV_DIR="$HOME/.local/share/ytui/venv"
echo -e "${CYAN}[3/4] Creating virtual environment at ${VENV_DIR}...${NC}"

python3 -m venv "$VENV_DIR"
"$VENV_DIR/bin/pip" install --upgrade pip --quiet
"$VENV_DIR/bin/pip" install -e "$SOURCE_DIR" --quiet

# 4. Create single-command symlink
echo -e "${CYAN}[4/4] Creating single-command symlink in ${BIN_DIR}/ytui...${NC}"

ln -sf "$VENV_DIR/bin/ytui" "$BIN_DIR/ytui"

# Ensure ~/.local/bin is in PATH
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo -e "${YELLOW}\nNote: ${BIN_DIR} is not currently in your PATH.${NC}"
    echo -e "Add this line to your ~/.bashrc or ~/.zshrc:"
    echo -e "    ${CYAN}export PATH=\"\$HOME/.local/bin:\$PATH\"${NC}"
fi

echo -e "\n${GREEN}=====================================================${NC}"
echo -e "${GREEN}  ytui successfully installed! 🎉${NC}"
echo -e "${GREEN}=====================================================${NC}"
echo -e "Run the app anywhere with just one command:\n"
echo -e "    ${CYAN}ytui${NC}\n"
