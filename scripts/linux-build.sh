#!/bin/bash
# Linux服务器环境下的构建脚本

echo "检查Linux环境下构建所需的依赖..."

# 检测Linux发行版
if [ -f /etc/os-release ]; then
    . /etc/os-release
    OS=$NAME
    VER=$VERSION_ID
else
    echo "无法检测Linux发行版，将尝试通用安装方式"
    OS="Unknown"
fi

# 安装依赖函数
install_deps_debian() {
    echo "检测到 Debian/Ubuntu 系统，准备安装依赖..."
    sudo apt-get update
    sudo apt-get install -y cmake pkg-config libssl-dev build-essential git librdkafka-dev
}

install_deps_rhel() {
    echo "检测到 RHEL/CentOS/Fedora 系统，准备安装依赖..."
    sudo yum -y update
    sudo yum -y install cmake pkgconfig openssl-devel gcc gcc-c++ make git librdkafka-devel
}

install_deps_suse() {
    echo "检测到 SUSE/openSUSE 系统，准备安装依赖..."
    sudo zypper refresh
    sudo zypper install -y cmake pkg-config libopenssl-devel gcc gcc-c++ make git librdkafka-devel
}

install_deps_arch() {
    echo "检测到 Arch 系统，准备安装依赖..."
    sudo pacman -Sy
    sudo pacman -S --noconfirm cmake pkgconfig openssl gcc git librdkafka
}

# 根据发行版安装依赖
if [[ "$OS" == *"Ubuntu"* ]] || [[ "$OS" == *"Debian"* ]]; then
    read -p "需要安装依赖，是否继续? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        install_deps_debian
    else
        echo "请手动安装所需依赖后再次运行此脚本"
        exit 1
    fi
elif [[ "$OS" == *"Red Hat"* ]] || [[ "$OS" == *"CentOS"* ]] || [[ "$OS" == *"Fedora"* ]]; then
    read -p "需要安装依赖，是否继续? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        install_deps_rhel
    else
        echo "请手动安装所需依赖后再次运行此脚本"
        exit 1
    fi
elif [[ "$OS" == *"SUSE"* ]] || [[ "$OS" == *"openSUSE"* ]]; then
    read -p "需要安装依赖，是否继续? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        install_deps_suse
    else
        echo "请手动安装所需依赖后再次运行此脚本"
        exit 1
    fi
elif [[ "$OS" == *"Arch"* ]]; then
    read -p "需要安装依赖，是否继续? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        install_deps_arch
    else
        echo "请手动安装所需依赖后再次运行此脚本"
        exit 1
    fi
else
    echo "未能识别的Linux发行版，请手动安装以下依赖:"
    echo "- cmake"
    echo "- pkg-config"
    echo "- openssl开发库"
    echo "- gcc/g++"
    echo "- make"
    echo "- git"
    echo "- librdkafka开发库"
    read -p "是否继续构建? (y/n) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

echo "所有依赖已满足，准备构建..."

# 询问构建方式
echo "选择构建方式:"
echo "1. 使用系统librdkafka (推荐，更快速)"
echo "2. 使用内置cmake构建 (更稳定，但构建时间更长)"
read -p "请选择 [1/2]: " BUILD_CHOICE

if [ "$BUILD_CHOICE" = "1" ]; then
    echo "使用系统librdkafka构建..."
    cargo build --features dynamic --no-default-features
else
    echo "使用内置cmake构建..."
    cargo build
fi

if [ $? -ne 0 ]; then
    echo "构建失败，可能原因:"
    echo "1. 可能需要更新Rust工具链: rustup update"
    echo "2. 尝试使用内置cmake构建: cargo build --features static --no-default-features"
    echo "3. 检查系统依赖是否正确安装"
else
    echo "构建成功!"
fi 