#!/usr/bin/env bash
# ==============================================================================
# build.sh — Build hoàn thiện ứng dụng Docker Manager (Tauri v2 + React 19 + Rust)
# ==============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

success() {
    echo -e "${GREEN}[OK]${NC} $*"
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

info "1. Build Frontend assets với Vite..."
pnpm build

info "2. Build Tauri Release Binary..."
cargo build --release --manifest-path src-tauri/Cargo.toml

BIN_PATH="$ROOT_DIR/src-tauri/target/release/docker-manager"
if [ -f "$BIN_PATH" ]; then
    success "Build hoàn tất thành công!"
    info "Binary path: $BIN_PATH"
    info "Kích thước binary: $(du -h "$BIN_PATH" | cut -f1)"
else
    error "Build thất bại, không tìm thấy file binary tại $BIN_PATH"
    exit 1
fi
