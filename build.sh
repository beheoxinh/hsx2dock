#!/usr/bin/env bash
# ==============================================================================
# build.sh — Build hoàn thiện ứng dụng Hsx2Dock (Tauri v2 + React 19 + Rust)
# ==============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

success() {
    echo -e "${GREEN}[OK]${NC} $*"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1; then
    error "Không tìm thấy 'cargo'. Hãy cài đặt Rust toolchain trước."
    exit 1
fi

if ! command -v pnpm >/dev/null 2>&1; then
    error "Không tìm thấy 'pnpm'. Hãy cài đặt pnpm trước."
    exit 1
fi

info "Tiến hành build ứng dụng với Tauri CLI (Release mode, nhúng offline assets)..."
pnpm tauri build --no-bundle

# Tauri chỉ build binary GUI; MCP server (stdio) là binary riêng, agent
# khởi chạy trực tiếp nên phải build tường minh.
info "Build MCP server (stdio) cho agent AI..."
cargo build --release --manifest-path "$ROOT_DIR/src-tauri/Cargo.toml" --bin hsx2dock-mcp

BIN_PATH="$ROOT_DIR/src-tauri/target/release/hsx2dock"
MCP_BIN_PATH="$ROOT_DIR/src-tauri/target/release/hsx2dock-mcp"
if [ -f "$MCP_BIN_PATH" ]; then
    info "MCP binary path: $MCP_BIN_PATH"
else
    warn "Không build được hsx2dock-mcp — MCP qua stdio sẽ không khả dụng."
fi

if [ -f "$BIN_PATH" ]; then
    success "Build hoàn tất thành công!"
    info "Binary path: $BIN_PATH"
    info "Kích thước binary: $(du -h "$BIN_PATH" | cut -f1)"
else
    error "Build thất bại, không tìm thấy file binary tại $BIN_PATH"
    exit 1
fi
