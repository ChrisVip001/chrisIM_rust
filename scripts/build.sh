#!/bin/bash
# RustIM 构建脚本：自动检测操作系统、安装编译依赖并构建
#
# 用法：
#   ./scripts/build.sh          # 自动检测 macOS / Linux
#   ./scripts/build.sh release  # 以 release 模式构建
set -e

MODE="debug"
BUILD_FLAGS=()
if [ "$1" = "release" ]; then
    MODE="release"
    BUILD_FLAGS=(--release)
fi

ask_continue() {
    read -p "$1 (y/n) " -n 1 -r
    echo
    [[ $REPLY =~ ^[Yy]$ ]]
}

# ---------- macOS ----------
build_macos() {
    echo "检查 macOS 构建依赖..."
    command -v brew &> /dev/null || {
        echo "错误: 未找到 Homebrew，请先安装: https://brew.sh"
        exit 1
    }

    local missing=()
    for dep in cmake pkg-config openssl librdkafka; do
        brew list "$dep" &> /dev/null || missing+=("$dep")
    done

    if [ ${#missing[@]} -ne 0 ]; then
        echo "需要安装依赖: ${missing[*]}"
        ask_continue "是否立即安装?" && brew install "${missing[@]}" || {
            echo "请手动安装后重试"; exit 1;
        }
    fi

    # 使用 Homebrew 的 openssl / librdkafka
    export OPENSSL_DIR="$(brew --prefix openssl)"
    export LIBRDKAFKA_DIR="$(brew --prefix librdkafka)"
    cargo build "${BUILD_FLAGS[@]}"
}

# ---------- Linux ----------
build_linux() {
    echo "检查 Linux 发行版..."
    local os="Unknown"
    [ -f /etc/os-release ] && { . /etc/os-release; os="$NAME"; }

    install_debian()  { sudo apt-get update && sudo apt-get install -y cmake pkg-config libssl-dev build-essential git librdkafka-dev; }
    install_rhel()    { sudo yum -y update && sudo yum -y install cmake pkgconfig openssl-devel gcc gcc-c++ make git librdkafka-devel; }
    install_suse()    { sudo zypper refresh && sudo zypper install -y cmake pkg-config libopenssl-devel gcc gcc-c++ make git librdkafka-devel; }
    install_arch()    { sudo pacman -Sy --noconfirm cmake pkgconfig openssl gcc git librdkafka; }

    local known=1
    case "$os" in
        *Ubuntu*|*Debian*)  install_debian ;;
        *Red\ Hat*|*CentOS*|*Fedora*)  install_rhel ;;
        *SUSE*|*openSUSE*)  install_suse ;;
        *Arch*)             install_arch ;;
        *)
            known=0
            echo "未能识别的发行版 ($os)，请手动安装: cmake pkg-config openssl开发库 gcc make git librdkafka开发库"
            ask_continue "依赖就绪，是否继续构建?" || exit 1
            ;;
    esac
    [ "$known" = 1 ] && ask_continue "需要安装系统依赖，是否继续?" || true

    echo "开始构建 ($MODE)..."
    # 默认使用系统 librdkafka（更快）；如需内置 cmake 构建请去掉 dynamic 特性
    cargo build --features dynamic --no-default-features "${BUILD_FLAGS[@]}" \
        || cargo build "${BUILD_FLAGS[@]}"
}

# ---------- 入口 ----------
case "$(uname -s)" in
    Darwin*)            build_macos ;;
    Linux*)             build_linux ;;
    CYGWIN*|MINGW*|MSYS*)
        echo "Windows 请使用 WSL 或 Docker 构建（见 Dockerfile）"
        exit 1
        ;;
    *)
        echo "未知操作系统: $(uname -s)"
        exit 1
        ;;
esac

echo "构建完成！"
