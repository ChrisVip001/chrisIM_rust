#!/bin/bash

# Docker Compose 安装脚本
# 适用于 OpenCloudOS 和其他 Linux 发行版

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

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

# 检查 Docker 是否已安装
check_docker() {
    if ! command -v docker &> /dev/null; then
        log_error "Docker 未安装，请先安装 Docker"
        log_info "运行: ./scripts/install-docker-opencloudos.sh"
        exit 1
    fi
    
    log_success "Docker 已安装: $(docker --version)"
}

# 检查权限
check_permissions() {
    if [[ $EUID -eq 0 ]]; then
        log_warning "正在使用 root 用户运行"
    else
        if ! sudo -n true 2>/dev/null; then
            log_info "此脚本需要 sudo 权限，请输入密码"
            sudo -v
        fi
    fi
}

# 安装 Docker Compose
install_docker_compose() {
    log_info "开始安装 Docker Compose..."
    
    # 检查是否已有 Docker Compose Plugin
    if docker compose version &> /dev/null; then
        log_success "Docker Compose Plugin 已可用"
        docker compose version
        return 0
    fi
    
    # 检查是否已有独立的 docker-compose
    if docker-compose --version &> /dev/null; then
        log_success "Docker Compose 独立版本已可用"
        docker-compose --version
        return 0
    fi
    
    # 检测包管理器
    if command -v dnf &> /dev/null; then
        PKG_MANAGER="dnf"
    elif command -v yum &> /dev/null; then
        PKG_MANAGER="yum"
    elif command -v apt &> /dev/null; then
        PKG_MANAGER="apt"
    else
        PKG_MANAGER=""
    fi
    
    # 方法1: 尝试通过包管理器安装
    if [[ -n "$PKG_MANAGER" ]]; then
        log_info "尝试通过包管理器 ($PKG_MANAGER) 安装 Docker Compose..."
        
        case $PKG_MANAGER in
            "dnf"|"yum")
                if sudo $PKG_MANAGER install -y docker-compose 2>/dev/null; then
                    log_success "通过 $PKG_MANAGER 安装 Docker Compose 成功"
                    return 0
                fi
                
                if sudo $PKG_MANAGER install -y docker-compose-plugin 2>/dev/null; then
                    log_success "通过 $PKG_MANAGER 安装 Docker Compose Plugin 成功"
                    return 0
                fi
                ;;
            "apt")
                sudo apt update
                if sudo apt install -y docker-compose 2>/dev/null; then
                    log_success "通过 apt 安装 Docker Compose 成功"
                    return 0
                fi
                
                if sudo apt install -y docker-compose-plugin 2>/dev/null; then
                    log_success "通过 apt 安装 Docker Compose Plugin 成功"
                    return 0
                fi
                ;;
        esac
    fi
    
    # 方法2: 通过 pip 安装
    log_info "尝试通过 pip 安装 Docker Compose..."
    if command -v pip3 &> /dev/null; then
        if sudo pip3 install docker-compose 2>/dev/null; then
            log_success "通过 pip3 安装 Docker Compose 成功"
            return 0
        fi
    elif command -v pip &> /dev/null; then
        if sudo pip install docker-compose 2>/dev/null; then
            log_success "通过 pip 安装 Docker Compose 成功"
            return 0
        fi
    else
        log_info "pip 未安装，跳过 pip 安装方式"
    fi
    
    # 方法3: 下载二进制文件安装
    log_info "下载 Docker Compose 二进制文件..."
    
    # 获取最新版本
    log_info "获取 Docker Compose 最新版本..."
    COMPOSE_VERSION=$(curl -s https://api.github.com/repos/docker/compose/releases/latest | grep 'tag_name' | cut -d\" -f4 2>/dev/null)
    
    if [[ -z "$COMPOSE_VERSION" ]]; then
        COMPOSE_VERSION="v2.24.1"  # 备用版本
        log_warning "无法获取最新版本，使用备用版本: $COMPOSE_VERSION"
    else
        log_info "使用版本: $COMPOSE_VERSION"
    fi
    
    # 检测系统架构
    ARCH=$(uname -m)
    case $ARCH in
        x86_64)
            ARCH="x86_64"
            ;;
        aarch64|arm64)
            ARCH="aarch64"
            ;;
        armv7l)
            ARCH="armv7"
            ;;
        *)
            log_error "不支持的架构: $ARCH"
            return 1
            ;;
    esac
    
    # 构建下载 URL
    DOWNLOAD_URL="https://github.com/docker/compose/releases/download/${COMPOSE_VERSION}/docker-compose-linux-${ARCH}"
    log_info "下载地址: $DOWNLOAD_URL"
    
    # 创建临时目录
    TEMP_FILE="/tmp/docker-compose-${COMPOSE_VERSION}"
    
    # 尝试多个下载方式
    if command -v curl &> /dev/null; then
        log_info "使用 curl 下载..."
        if curl -L "$DOWNLOAD_URL" -o "$TEMP_FILE" 2>/dev/null; then
            log_success "curl 下载成功"
        else
            log_warning "curl 下载失败"
            TEMP_FILE=""
        fi
    elif command -v wget &> /dev/null; then
        log_info "使用 wget 下载..."
        if wget -O "$TEMP_FILE" "$DOWNLOAD_URL" 2>/dev/null; then
            log_success "wget 下载成功"
        else
            log_warning "wget 下载失败"
            TEMP_FILE=""
        fi
    else
        log_error "curl 和 wget 都不可用，无法下载"
        return 1
    fi
    
    if [[ -z "$TEMP_FILE" || ! -f "$TEMP_FILE" ]]; then
        log_error "下载 Docker Compose 失败"
        return 1
    fi
    
    # 验证下载的文件
    if [[ ! -s "$TEMP_FILE" ]]; then
        log_error "下载的文件为空"
        rm -f "$TEMP_FILE"
        return 1
    fi
    
    # 安装到系统目录
    log_info "安装 Docker Compose 到系统目录..."
    sudo mv "$TEMP_FILE" /usr/local/bin/docker-compose
    sudo chmod +x /usr/local/bin/docker-compose
    
    # 创建符号链接
    sudo ln -sf /usr/local/bin/docker-compose /usr/bin/docker-compose
    
    # 验证安装
    if docker-compose --version &> /dev/null; then
        log_success "Docker Compose 安装成功"
        docker-compose --version
        return 0
    else
        log_error "Docker Compose 安装验证失败"
        return 1
    fi
}

# 显示使用信息
show_usage_info() {
    echo ""
    log_success "Docker Compose 安装完成！"
    echo ""
    log_info "验证安装:"
    echo "  docker-compose --version"
    echo "  docker compose version"
    echo ""
    log_info "基本使用:"
    echo "  docker-compose up -d        # 启动服务"
    echo "  docker-compose down         # 停止服务"
    echo "  docker-compose logs         # 查看日志"
    echo "  docker-compose ps           # 查看状态"
    echo ""
    log_info "RustIM 项目使用:"
    echo "  cd /path/to/rust-im"
    echo "  docker-compose up -d        # 启动 RustIM 服务"
    echo ""
}

# 主函数
main() {
    echo "========================================"
    echo "      Docker Compose 安装脚本"
    echo "========================================"
    echo ""
    
    # 检查 Docker
    check_docker
    
    # 检查权限
    check_permissions
    
    # 安装 Docker Compose
    install_docker_compose
    
    # 显示使用信息
    show_usage_info
}

# 执行主函数
main "$@" 