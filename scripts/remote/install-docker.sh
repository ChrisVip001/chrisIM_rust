#!/bin/bash

# Docker 环境安装脚本
# 支持 Ubuntu/Debian/CentOS/RHEL 系统

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 日志函数
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# 检测操作系统
detect_os() {
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        OS=$NAME
        VER=$VERSION_ID
    elif type lsb_release >/dev/null 2>&1; then
        OS=$(lsb_release -si)
        VER=$(lsb_release -sr)
    elif [[ -f /etc/redhat-release ]]; then
        OS="CentOS"
        VER=$(rpm -q --qf "%{VERSION}" $(rpm -q --whatprovides redhat-release))
    else
        log_error "无法检测操作系统"
        exit 1
    fi
    
    log_info "检测到操作系统: $OS $VER"
}

# 检查是否已安装 Docker
check_docker_installed() {
    if command -v docker &> /dev/null; then
        DOCKER_VERSION=$(docker --version | cut -d' ' -f3 | cut -d',' -f1)
        log_warning "Docker 已安装，版本: $DOCKER_VERSION"
        
        if command -v docker-compose &> /dev/null || docker compose version &> /dev/null; then
            if command -v docker-compose &> /dev/null; then
                COMPOSE_VERSION=$(docker-compose --version | cut -d' ' -f3 | cut -d',' -f1)
                log_warning "Docker Compose 已安装，版本: $COMPOSE_VERSION"
            else
                COMPOSE_VERSION=$(docker compose version --short)
                log_warning "Docker Compose (Plugin) 已安装，版本: $COMPOSE_VERSION"
            fi
            
            read -p "是否要重新安装 Docker? (y/N): " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                log_info "跳过 Docker 安装"
                return 0
            fi
        fi
    fi
    return 1
}

# 卸载旧版本 Docker
remove_old_docker() {
    log_info "卸载旧版本 Docker..."
    
    case $OS in
        "Ubuntu"|"Debian"*)
            sudo apt-get remove -y docker docker-engine docker.io containerd runc 2>/dev/null || true
            ;;
        "CentOS"*|"Red Hat"*|"Rocky"*|"AlmaLinux"*|"OpenCloudOS"*)
            sudo yum remove -y docker docker-client docker-client-latest docker-common docker-latest docker-latest-logrotate docker-logrotate docker-engine 2>/dev/null || true
            ;;
    esac
    
    log_success "旧版本 Docker 卸载完成"
}

# 安装 Docker (Ubuntu/Debian)
install_docker_ubuntu() {
    log_info "在 Ubuntu/Debian 上安装 Docker..."
    
    # 更新包索引
    sudo apt-get update
    
    # 安装必要的包
    sudo apt-get install -y \
        ca-certificates \
        curl \
        gnupg \
        lsb-release
    
    # 添加 Docker 官方 GPG 密钥
    sudo mkdir -p /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    
    # 设置稳定版仓库
    echo \
        "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu \
        $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
    
    # 更新包索引
    sudo apt-get update
    
    # 安装 Docker Engine
    sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
    
    log_success "Docker 安装完成"
}

# 安装 Docker (CentOS/RHEL)
install_docker_centos() {
    log_info "在 CentOS/RHEL 上安装 Docker..."
    
    # 安装必要的包
    sudo yum install -y yum-utils
    
    # 添加 Docker 仓库
    sudo yum-config-manager \
        --add-repo \
        https://download.docker.com/linux/centos/docker-ce.repo
    
    # 安装 Docker Engine
    sudo yum install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
    
    log_success "Docker 安装完成"
}

# 安装 Docker (OpenCloudOS/CentOS/RHEL)
install_docker_opencloudos() {
    log_info "在 OpenCloudOS/CentOS/RHEL 上安装 Docker..."
    
    # 检测包管理器
    if command -v dnf &> /dev/null; then
        PKG_MANAGER="dnf"
    else
        PKG_MANAGER="yum"
    fi
    
    log_info "使用包管理器: $PKG_MANAGER"
    
    # 安装必要的包
    sudo $PKG_MANAGER install -y yum-utils device-mapper-persistent-data lvm2
    
    # 添加 Docker 仓库
    if [[ "$OS" == *"OpenCloudOS"* ]]; then
        # OpenCloudOS 使用 CentOS 8 的仓库
        sudo $PKG_MANAGER config-manager \
            --add-repo \
            https://download.docker.com/linux/centos/docker-ce.repo
    else
        sudo $PKG_MANAGER config-manager \
            --add-repo \
            https://download.docker.com/linux/centos/docker-ce.repo
    fi
    
    # 更新包索引
    sudo $PKG_MANAGER makecache
    
    # 安装 Docker Engine
    sudo $PKG_MANAGER install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
    
    log_success "Docker 安装完成"
}

# 配置 Docker
configure_docker() {
    log_info "配置 Docker..."
    
    # 启动 Docker 服务
    sudo systemctl start docker
    sudo systemctl enable docker
    
    # 将当前用户添加到 docker 组
    sudo usermod -aG docker $USER
    
    # 创建 Docker 配置目录
    sudo mkdir -p /etc/docker
    
    # 配置 Docker daemon
    cat << EOF | sudo tee /etc/docker/daemon.json
{
    "log-driver": "json-file",
    "log-opts": {
        "max-size": "100m",
        "max-file": "3"
    },
    "storage-driver": "overlay2",
    "registry-mirrors": [
        "https://docker.mirrors.ustc.edu.cn",
        "https://hub-mirror.c.163.com"
    ]
}
EOF
    
    # 重启 Docker 服务
    sudo systemctl restart docker
    
    log_success "Docker 配置完成"
}

# 安装 Docker Compose (独立版本)
install_docker_compose() {
    log_info "检查 Docker Compose..."
    
    # 检查是否已有 Docker Compose Plugin
    if docker compose version &> /dev/null; then
        log_success "Docker Compose Plugin 已可用"
        return 0
    fi
    
    # 安装独立的 docker-compose
    log_info "安装 Docker Compose 独立版本..."
    
    COMPOSE_VERSION=$(curl -s https://api.github.com/repos/docker/compose/releases/latest | grep 'tag_name' | cut -d\" -f4)
    
    sudo curl -L "https://github.com/docker/compose/releases/download/${COMPOSE_VERSION}/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
    
    sudo chmod +x /usr/local/bin/docker-compose
    
    # 创建符号链接
    sudo ln -sf /usr/local/bin/docker-compose /usr/bin/docker-compose
    
    log_success "Docker Compose 安装完成"
}

# 验证安装
verify_installation() {
    log_info "验证 Docker 安装..."
    
    # 检查 Docker 版本
    if docker --version; then
        log_success "Docker 安装成功"
    else
        log_error "Docker 安装失败"
        exit 1
    fi
    
    # 检查 Docker Compose 版本
    if docker-compose --version 2>/dev/null || docker compose version 2>/dev/null; then
        log_success "Docker Compose 安装成功"
    else
        log_error "Docker Compose 安装失败"
        exit 1
    fi
    
    # 测试 Docker 运行
    log_info "测试 Docker 运行..."
    if sudo docker run --rm hello-world; then
        log_success "Docker 运行测试成功"
    else
        log_error "Docker 运行测试失败"
        exit 1
    fi
}

# 显示安装后信息
show_post_install_info() {
    echo ""
    log_success "Docker 环境安装完成！"
    echo ""
    log_info "安装信息:"
    echo "  Docker 版本: $(docker --version)"
    if command -v docker-compose &> /dev/null; then
        echo "  Docker Compose 版本: $(docker-compose --version)"
    else
        echo "  Docker Compose 版本: $(docker compose version --short)"
    fi
    echo ""
    log_warning "重要提示:"
    echo "  1. 当前用户已添加到 docker 组"
    echo "  2. 请重新登录或运行 'newgrp docker' 以使组权限生效"
    echo "  3. 之后可以不使用 sudo 运行 docker 命令"
    echo ""
    log_info "下一步:"
    echo "  1. 重新登录系统或运行: newgrp docker"
    echo "  2. 测试 Docker: docker run hello-world"
    echo "  3. 部署 RustIM: ./scripts/deploy.sh"
    echo ""
}

# 主函数
main() {
    echo "========================================"
    echo "       RustIM Docker 环境安装脚本"
    echo "========================================"
    echo ""
    
    # 检查是否为 root 用户
    if [[ $EUID -eq 0 ]]; then
        log_error "请不要使用 root 用户运行此脚本"
        exit 1
    fi
    
    # 检查 sudo 权限
    if ! sudo -n true 2>/dev/null; then
        log_info "此脚本需要 sudo 权限，请输入密码"
        sudo -v
    fi
    
    # 检测操作系统
    detect_os
    
    # 检查是否已安装
    if check_docker_installed; then
        verify_installation
        show_post_install_info
        exit 0
    fi
    
    # 卸载旧版本
    remove_old_docker
    
    # 根据操作系统安装 Docker
    case $OS in
        "Ubuntu"|"Debian"*)
            install_docker_ubuntu
            ;;
        "CentOS"*|"Red Hat"*|"Rocky"*|"AlmaLinux"*)
            install_docker_opencloudos
            ;;
        "OpenCloudOS"*)
            install_docker_opencloudos
            ;;
        *)
            log_error "不支持的操作系统: $OS"
            log_info "支持的操作系统: Ubuntu, Debian, CentOS, RHEL, Rocky Linux, AlmaLinux, OpenCloudOS"
            exit 1
            ;;
    esac
    
    # 配置 Docker
    configure_docker
    
    # 安装 Docker Compose
    install_docker_compose
    
    # 验证安装
    verify_installation
    
    # 显示安装后信息
    show_post_install_info
}

# 执行主函数
main "$@" 