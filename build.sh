#!/usr/bin/env bash
# ==============================================================================
# build.sh — Build hoàn thiện tất cả các thành phần của dự án
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

# Lấy thư mục gốc của project
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# Nạp Rust environment nếu tồn tại
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1; then
    error "Không tìm thấy 'cargo'. Hãy chạy ./prepare.sh trước."
    exit 1
fi

info "Kiểm tra file cấu hình và icon..."
# Kiểm tra nếu có icon nguồn /mnt/SubSystem/Downloads/google-container-optimized-os.svg
SOURCE_ICON="/mnt/SubSystem/Downloads/google-container-optimized-os.svg"
APP_ICON="$ROOT_DIR/data/icons/com.github.docker_manager.svg"

if [ -f "$SOURCE_ICON" ]; then
    info "Đang đồng bộ icon từ $SOURCE_ICON..."
    /bin/cp -f "$SOURCE_ICON" "$APP_ICON"
fi

if [ ! -f "$APP_ICON" ]; then
    error "Không tìm thấy file icon ứng dụng tại: $APP_ICON"
    exit 1
fi

info "Kiểm tra tính hợp lệ của file .desktop..."
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$ROOT_DIR/data/com.github.docker_manager.desktop"
    success "File .desktop hợp lệ."
fi

info "Đang tiến hành build release binary với Cargo..."
cargo build --release

BIN_PATH="$ROOT_DIR/target/release/docker-manager"
if [ -f "$BIN_PATH" ]; then
    success "Build hoàn tất thành công!"
    info "Binary path: $BIN_PATH"
    info "Kích thước binary: $(du -h "$BIN_PATH" | cut -f1)"
else
    error "Build thất bại, không tìm thấy file binary tại $BIN_PATH"
    exit 1
fi
