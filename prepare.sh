#!/usr/bin/env bash
# ==============================================================================
# prepare.sh — Cài đặt các công cụ và thư viện phụ thuộc để phát triển dự án
# Hỗ trợ: Fedora / Nobara / RHEL / CentOS (và các bản phân phối dựa trên Fedora)
# cũng như Ubuntu / Debian / Arch Linux nếu cần chạy trên máy khác.
# ==============================================================================

set -euo pipefail

# Màu hiển thị
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

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

# 1. Kiểm tra quyền sudo nếu cần cài gói hệ thống
SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    else
        error "Script cần quyền root hoặc sudo để cài đặt các package hệ thống."
        exit 1
    fi
fi

# 2. Nhận diện Package Manager và cài đặt gói hệ thống
install_system_deps() {
    info "Đang phát hiện hệ điều hành và trình quản lý gói..."

    if command -v dnf5 >/dev/null 2>&1; then
        info "Phát hiện dnf5 (Fedora / Nobara). Đang cài đặt thư viện phát triển..."
        $SUDO dnf5 install -y \
            gcc \
            gcc-c++ \
            glibc-devel \
            pkgconf-pkg-config \
            openssl-devel \
            glib2-devel \
            gtk4-devel \
            libadwaita-devel \
            desktop-file-utils \
            hicolor-icon-theme \
            git \
            curl
    elif command -v dnf >/dev/null 2>&1; then
        info "Phát hiện dnf (Fedora / RHEL / CentOS). Đang cài đặt thư viện phát triển..."
        $SUDO dnf install -y \
            gcc \
            gcc-c++ \
            glibc-devel \
            pkgconf-pkg-config \
            openssl-devel \
            glib2-devel \
            gtk4-devel \
            libadwaita-devel \
            desktop-file-utils \
            hicolor-icon-theme \
            git \
            curl
    elif command -v apt-get >/dev/null 2>&1; then
        info "Phát hiện apt (Debian / Ubuntu). Đang cài đặt thư viện phát triển..."
        $SUDO apt-get update -y
        $SUDO apt-get install -y \
            build-essential \
            pkg-config \
            libssl-dev \
            libglib2.0-dev \
            libgtk-4-dev \
            libadwaita-1-dev \
            desktop-file-utils \
            hicolor-icon-theme \
            git \
            curl
    elif command -v pacman >/dev/null 2>&1; then
        info "Phát hiện pacman (Arch Linux). Đang cài đặt thư viện phát triển..."
        $SUDO pacman -S --needed --noconfirm \
            base-devel \
            pkgconf \
            openssl \
            glib2 \
            gtk4 \
            libadwaita \
            desktop-file-utils \
            hicolor-icon-theme \
            git \
            curl
    else
        warn "Không tìm thấy trình quản lý gói được hỗ trợ (dnf, apt, pacman)."
        warn "Vui lòng tự cài đặt: gcc, pkg-config, openssl-devel, glib2-devel, gtk4-devel, libadwaita-devel"
    fi
}

# 3. Kiểm tra hoặc cài đặt Rust / Cargo
install_rust() {
    if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
        success "Rust và Cargo đã được cài đặt: $(rustc --version)"
    else
        info "Chưa tìm thấy Rust/Cargo. Đang cài đặt thông qua rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
        success "Đã cài đặt Rust thành công: $(rustc --version)"
    fi
}

# 4. Kiểm tra Docker (tùy chọn nhưng cần thiết khi chạy app)
check_docker() {
    if command -v docker >/dev/null 2>&1; then
        success "Docker CLI đã sẵn sàng: $(docker --version)"
    else
        warn "Docker CLI chưa được cài đặt trên máy này."
        warn "Ứng dụng vẫn có thể build được, nhưng bạn cần cài Docker để quản lý container."
    fi
}

# 5. Kiểm tra pkg-config với các thư viện GTK4 & Libadwaita
verify_dev_env() {
    info "Đang kiểm tra pkg-config cho GTK4 và libadwaita-1..."
    local missing=0

    for pkg in glib-2.0 gtk4 libadwaita-1; do
        if pkg-config --exists "$pkg"; then
            success "pkg-config tìm thấy: $pkg ($(pkg-config --modversion "$pkg"))"
        else
            error "Không tìm thấy thư viện: $pkg"
            missing=1
        fi
    done

    if [ "$missing" -ne 0 ]; then
        error "Thiếu thư viện C/GTK cần thiết. Hãy kiểm tra lại log bên trên."
        exit 1
    fi
}

main() {
    echo "=========================================================="
    echo "      Hsx2Dock — Chuẩn bị môi trường phát triển       "
    echo "=========================================================="
    install_system_deps
    install_rust
    check_docker
    verify_dev_env
    echo "=========================================================="
    success "Hoàn tất! Môi trường đã sẵn sàng để phát triển và build dự án."
    echo "=========================================================="
}

main "$@"
