#!/usr/bin/env bash
# ==============================================================================
# install.sh — Quy trình build và cài đặt ứng dụng vào hệ thống (Tauri v2)
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

APP_ID="com.github.docker-manager"
SOURCE_ICON="/mnt/SubSystem/Downloads/google-container-optimized-os.svg"
LOCAL_ICON="$ROOT_DIR/data/icons/${APP_ID}.svg"

echo "=========================================================="
echo "   Docker Manager — Build & Cài đặt hệ thống (Tauri v2)   "
echo "=========================================================="

# 1. Đảm bảo icon có sẵn
if [ -f "$SOURCE_ICON" ]; then
    info "Đang sao chép icon từ $SOURCE_ICON..."
    /bin/cp -f "$SOURCE_ICON" "$LOCAL_ICON"
fi

# 2. Chạy quy trình build
info "Đang chạy script build.sh..."
"$ROOT_DIR/build.sh"

BIN_SRC="$ROOT_DIR/src-tauri/target/release/docker-manager"
if [ ! -f "$BIN_SRC" ]; then
    error "Không tìm thấy file binary sau khi build: $BIN_SRC"
    exit 1
fi

# 3. Thiết lập các thư mục cài đặt người dùng (User prefix XDG)
USER_BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
USER_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
USER_APPS_DIR="$USER_DATA_DIR/applications"
USER_ICONS_DIR="$USER_DATA_DIR/icons/hicolor/scalable/apps"

info "Chuẩn bị thư mục đích:"
info " - Binary:  $USER_BIN_DIR"
info " - Desktop: $USER_APPS_DIR"
info " - Icon:    $USER_ICONS_DIR"

mkdir -p "$USER_BIN_DIR"
mkdir -p "$USER_APPS_DIR"
mkdir -p "$USER_ICONS_DIR"

# 4. Cài đặt binary
info "Đang cài đặt binary 'docker-manager' vào $USER_BIN_DIR/..."
install -m 755 "$BIN_SRC" "$USER_BIN_DIR/docker-manager"
success "Đã cài đặt binary thành công."

# 5. Cài đặt icon SVG
if [ -f "$LOCAL_ICON" ]; then
    info "Đang cài đặt icon '${APP_ID}.svg' vào $USER_ICONS_DIR/..."
    install -m 644 "$LOCAL_ICON" "$USER_ICONS_DIR/${APP_ID}.svg"
    success "Đã cài đặt icon ứng dụng."
fi

# 6. Cài đặt file .desktop (Dọn dẹp các desktop entry cũ không đồng bộ)
info "Đang dọn dẹp các desktop entry cũ..."
rm -f "$USER_APPS_DIR/com.github.docker_manager.desktop"

info "Đang tạo và cài đặt file .desktop vào $USER_APPS_DIR/..."
cat << EOF > "$USER_APPS_DIR/${APP_ID}.desktop"
[Desktop Entry]
Type=Application
Version=1.5
Name=Docker Manager
GenericName=Docker Desktop Suite
Comment=Modern Cross-platform Desktop Suite to manage Docker containers, images, volumes and networks
Exec=docker-manager
Icon=${APP_ID}
Terminal=false
Categories=Development;
Keywords=docker;containers;images;volumes;networks;
StartupWMClass=docker-manager
EOF

chmod 644 "$USER_APPS_DIR/${APP_ID}.desktop"

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$USER_APPS_DIR/${APP_ID}.desktop"
fi
success "Đã cài đặt file desktop entry: $USER_APPS_DIR/${APP_ID}.desktop"

# 7. Cập nhật cache của GNOME / Desktop database
info "Đang làm mới cache ứng dụng và icon..."
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$USER_APPS_DIR" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$USER_DATA_DIR/icons/hicolor" || true
fi

echo "=========================================================="
success "CÀI ĐẶT THÀNH CÔNG!"
echo "Ứng dụng Docker Manager (Tauri) đã sẵn sàng trên hệ thống."
echo "Bạn có thể mở ứng dụng bằng cách:"
echo " 1. Tìm 'Docker Manager' trong App Launcher (phím Super)"
echo " 2. Hoặc chạy lệnh trong terminal: docker-manager"
echo "=========================================================="
