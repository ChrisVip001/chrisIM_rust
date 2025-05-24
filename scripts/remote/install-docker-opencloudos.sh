#!/bin/bash

# OpenCloudOS Docker 安装脚本
# 专门为腾讯云 OpenCloudOS 系统设计

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

# 检查系统
check_system() {
    log_info "检查系统信息..."
    
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        log_info "操作系统: $NAME $VERSION_ID"
    else
        log_error "无法检测操作系统"
        exit 1
    fi
    
    # 检查架构
    ARCH=$(uname -m)
    log_info "系统架构: $ARCH"
    
    if [[ "$ARCH" != "x86_64" && "$ARCH" != "aarch64" ]]; then
        log_error "不支持的架构: $ARCH"
        exit 1
    fi
}

# 检查权限
check_permissions() {
    if [[ $EUID -eq 0 ]]; then
        log_error "请不要使用 root 用户运行此脚本"
        exit 1
    fi
    
    if ! sudo -n true 2>/dev/null; then
        log_info "此脚本需要 sudo 权限，请输入密码"
        sudo -v
    fi
}

# 卸载旧版本
remove_old_docker() {
    log_info "卸载旧版本 Docker..."
    
    # 检测包管理器
    if command -v dnf &> /dev/null; then
        PKG_MANAGER="dnf"
    else
        PKG_MANAGER="yum"
    fi
    
    sudo $PKG_MANAGER remove -y \
        docker \
        docker-client \
        docker-client-latest \
        docker-common \
        docker-latest \
        docker-latest-logrotate \
        docker-logrotate \
        docker-engine \
        podman \
        runc 2>/dev/null || true
    
    log_success "旧版本清理完成"
}

# 安装 Docker
install_docker() {
    log_info "开始安装 Docker..."
    
    # 检测包管理器
    if command -v dnf &> /dev/null; then
        PKG_MANAGER="dnf"
    else
        PKG_MANAGER="yum"
    fi
    
    log_info "使用包管理器: $PKG_MANAGER"
    
    # 更新系统
    log_info "更新系统包..."
    sudo $PKG_MANAGER update -y
    
    # 安装必要的包 - 针对 OpenCloudOS 优化
    log_info "安装依赖包..."
    
    # 尝试安装不同的包名
    if [[ "$PKG_MANAGER" == "dnf" ]]; then
        # DNF 包管理器的包名
        sudo $PKG_MANAGER install -y \
            dnf-utils \
            device-mapper-persistent-data \
            lvm2 \
            curl \
            wget \
            tar \
            gzip || {
            log_warning "使用 dnf-utils 失败，尝试其他包名..."
            sudo $PKG_MANAGER install -y \
                yum-utils \
                device-mapper-persistent-data \
                lvm2 \
                curl \
                wget \
                tar \
                gzip
        }
    else
        # YUM 包管理器 - 尝试多种包名
        if ! sudo $PKG_MANAGER install -y yum-utils 2>/dev/null; then
            log_warning "yum-utils 安装失败，尝试 dnf-utils..."
            if ! sudo $PKG_MANAGER install -y dnf-utils 2>/dev/null; then
                log_warning "dnf-utils 也失败，尝试 yum-config-manager..."
                sudo $PKG_MANAGER install -y \
                    python3-dnf-plugins-core \
                    curl \
                    wget \
                    tar \
                    gzip || {
                    log_error "无法安装必要的工具包"
                    exit 1
                }
            fi
        fi
        
        # 安装其他依赖
        sudo $PKG_MANAGER install -y \
            device-mapper-persistent-data \
            lvm2 \
            curl \
            wget \
            tar \
            gzip 2>/dev/null || true
    fi
    
    # 检查是否有 config-manager 命令
    if ! command -v yum-config-manager &> /dev/null && ! command -v dnf &> /dev/null; then
        log_warning "config-manager 不可用，尝试手动添加仓库..."
        
        # 手动创建 Docker 仓库文件
        sudo tee /etc/yum.repos.d/docker-ce.repo > /dev/null <<EOF
[docker-ce-stable]
name=Docker CE Stable - \$basearch
baseurl=https://download.docker.com/linux/centos/8/\$basearch/stable
enabled=1
gpgcheck=1
gpgkey=https://download.docker.com/linux/centos/gpg

[docker-ce-stable-debuginfo]
name=Docker CE Stable - Debuginfo \$basearch
baseurl=https://download.docker.com/linux/centos/8/debug-\$basearch/stable
enabled=0
gpgcheck=1
gpgkey=https://download.docker.com/linux/centos/gpg

[docker-ce-stable-source]
name=Docker CE Stable - Sources
baseurl=https://download.docker.com/linux/centos/8/source/stable
enabled=0
gpgcheck=1
gpgkey=https://download.docker.com/linux/centos/gpg
EOF
        
        log_info "手动创建 Docker 仓库文件完成"
    else
        # 添加 Docker 官方仓库
        log_info "添加 Docker 仓库..."
        if command -v dnf &> /dev/null; then
            sudo dnf config-manager \
                --add-repo \
                https://download.docker.com/linux/centos/docker-ce.repo
        else
            sudo yum-config-manager \
                --add-repo \
                https://download.docker.com/linux/centos/docker-ce.repo 2>/dev/null || {
                log_warning "yum-config-manager 失败，使用手动方式..."
                sudo tee /etc/yum.repos.d/docker-ce.repo > /dev/null <<EOF
[docker-ce-stable]
name=Docker CE Stable - \$basearch
baseurl=https://download.docker.com/linux/centos/8/\$basearch/stable
enabled=1
gpgcheck=1
gpgkey=https://download.docker.com/linux/centos/gpg
EOF
            }
        fi
    fi
    
    # 导入 GPG 密钥
    log_info "导入 Docker GPG 密钥..."
    sudo rpm --import https://download.docker.com/linux/centos/gpg 2>/dev/null || true
    
    # 更新包缓存
    log_info "更新包缓存..."
    sudo $PKG_MANAGER makecache 2>/dev/null || sudo $PKG_MANAGER clean all
    
    # 安装 Docker - 尝试不同的包组合
    log_info "安装 Docker Engine..."
    
    # 首先尝试完整安装
    if ! sudo $PKG_MANAGER install -y \
        docker-ce \
        docker-ce-cli \
        containerd.io \
        docker-buildx-plugin \
        docker-compose-plugin 2>/dev/null; then
        
        log_warning "完整安装失败，尝试基础安装..."
        
        # 尝试基础安装
        if ! sudo $PKG_MANAGER install -y \
            docker-ce \
            docker-ce-cli \
            containerd.io 2>/dev/null; then
            
            log_warning "官方包安装失败，尝试系统包..."
            
            # 最后尝试系统自带的 Docker
            sudo $PKG_MANAGER install -y docker || {
                log_error "所有 Docker 安装方式都失败了"
                exit 1
            }
        fi
    fi
    
    log_success "Docker 安装完成"
}

# 配置 Docker
configure_docker() {
    log_info "配置 Docker..."
    
    # 启动并启用 Docker 服务
    sudo systemctl start docker
    sudo systemctl enable docker
    
    # 将当前用户添加到 docker 组
    sudo usermod -aG docker $USER
    
    # 创建 Docker 配置目录
    sudo mkdir -p /etc/docker
    
    # 配置 Docker daemon（使用国内镜像源）
    log_info "配置 Docker daemon..."
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
        "https://hub-mirror.c.163.com",
        "https://mirror.ccs.tencentyun.com"
    ],
    "exec-opts": ["native.cgroupdriver=systemd"],
    "live-restore": true
}
EOF
    
    # 重启 Docker 服务
    sudo systemctl restart docker
    
    log_success "Docker 配置完成"
}

# 安装 Docker Compose（如果需要）
install_docker_compose() {
    log_info "检查 Docker Compose..."
    
    # 检查是否已有 Docker Compose Plugin
    if docker compose version &> /dev/null; then
        log_success "Docker Compose Plugin 已可用"
        return 0
    fi
    
    # 检查是否已有独立的 docker-compose
    if docker-compose --version &> /dev/null; then
        log_success "Docker Compose 独立版本已可用"
        return 0
    fi
    
    log_info "安装 Docker Compose..."
    
    # 方法1: 尝试通过包管理器安装
    log_info "尝试通过包管理器安装 Docker Compose..."
    if command -v dnf &> /dev/null; then
        PKG_MANAGER="dnf"
    else
        PKG_MANAGER="yum"
    fi
    
    if sudo $PKG_MANAGER install -y docker-compose 2>/dev/null; then
        log_success "通过包管理器安装 Docker Compose 成功"
        return 0
    fi
    
    # 方法2: 尝试安装 Docker Compose Plugin
    log_info "尝试安装 Docker Compose Plugin..."
    if sudo $PKG_MANAGER install -y docker-compose-plugin 2>/dev/null; then
        log_success "Docker Compose Plugin 安装成功"
        return 0
    fi
    
    # 方法3: 通过 pip 安装
    log_info "尝试通过 pip 安装 Docker Compose..."
    if command -v pip3 &> /dev/null || sudo $PKG_MANAGER install -y python3-pip 2>/dev/null; then
        if sudo pip3 install docker-compose 2>/dev/null; then
            log_success "通过 pip 安装 Docker Compose 成功"
            return 0
        fi
    fi
    
    # 方法4: 下载二进制文件安装
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
        aarch64)
            ARCH="aarch64"
            ;;
        *)
            log_error "不支持的架构: $ARCH"
            return 1
            ;;
    esac
    
    # 下载并安装
    DOWNLOAD_URL="https://github.com/docker/compose/releases/download/${COMPOSE_VERSION}/docker-compose-linux-${ARCH}"
    log_info "下载地址: $DOWNLOAD_URL"
    
    # 尝试多个下载方式
    if ! sudo curl -L "$DOWNLOAD_URL" -o /usr/local/bin/docker-compose 2>/dev/null; then
        log_warning "curl 下载失败，尝试 wget..."
        if ! sudo wget -O /usr/local/bin/docker-compose "$DOWNLOAD_URL" 2>/dev/null; then
            log_error "下载 Docker Compose 失败"
            return 1
        fi
    fi
    
    # 设置执行权限
    sudo chmod +x /usr/local/bin/docker-compose
    
    # 创建符号链接
    sudo ln -sf /usr/local/bin/docker-compose /usr/bin/docker-compose
    
    # 验证安装
    if docker-compose --version &> /dev/null; then
        log_success "Docker Compose 二进制安装成功"
        return 0
    else
        log_error "Docker Compose 安装验证失败"
        return 1
    fi
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
        log_warning "Docker Compose 安装可能有问题"
    fi
    
    # 检查 Docker 服务状态
    if sudo systemctl is-active docker &>/dev/null; then
        log_success "Docker 服务运行正常"
    else
        log_error "Docker 服务未运行"
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
    log_success "OpenCloudOS Docker 环境安装完成！"
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
    log_info "如果遇到问题，请检查："
    echo "  - 防火墙设置: sudo systemctl status firewalld"
    echo "  - SELinux 状态: getenforce"
    echo "  - Docker 日志: sudo journalctl -u docker"
    echo ""
}

# 主函数
main() {
    echo "========================================"
    echo "    OpenCloudOS Docker 环境安装脚本"
    echo "========================================"
    echo ""
    
    # 检查系统
    check_system
    
    # 检查权限
    check_permissions
    
    # 卸载旧版本
    remove_old_docker
    
    # 安装 Docker
    install_docker
    
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