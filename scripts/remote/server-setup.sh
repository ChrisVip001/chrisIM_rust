#!/bin/bash

# RustIM 服务器初始化脚本
# 用于在云服务器上进行初始化设置

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# 配置变量
PROJECT_NAME="rustim"
DEPLOY_USER=${DEPLOY_USER:-"rustim"}
PROJECT_REPO=${PROJECT_REPO:-"https://github.com/ChrisVip001/chrisIM_rust"}
SSH_KEY_PATH=${SSH_KEY_PATH:-""}

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
    
    # 检查是否为 root 用户
    if [[ $EUID -ne 0 ]]; then
        log_error "此脚本需要 root 权限运行"
        exit 1
    fi
    
    log_success "系统检查完成"
}

# 更新系统
update_system() {
    log_info "更新系统包..."
    
    if command -v dnf &> /dev/null; then
        dnf update -y
        dnf install -y curl wget git vim htop tree unzip
    elif command -v yum &> /dev/null; then
        yum update -y
        yum install -y curl wget git vim htop tree unzip
    elif command -v apt &> /dev/null; then
        apt update
        apt upgrade -y
        apt install -y curl wget git vim htop tree unzip
    else
        log_error "不支持的包管理器"
        exit 1
    fi
    
    log_success "系统更新完成"
}

# 创建部署用户
create_deploy_user() {
    log_info "创建部署用户: $DEPLOY_USER"
    
    # 检查用户是否已存在
    if id "$DEPLOY_USER" &>/dev/null; then
        log_warning "用户 $DEPLOY_USER 已存在"
    else
        # 创建用户
        useradd -m -s /bin/bash "$DEPLOY_USER"
        log_success "用户 $DEPLOY_USER 创建成功"
    fi
    
    # 添加到 sudo 组
    usermod -aG sudo "$DEPLOY_USER" 2>/dev/null || usermod -aG wheel "$DEPLOY_USER"
    
    # 设置 sudo 免密码
    echo "$DEPLOY_USER ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/$DEPLOY_USER"
    
    log_success "用户权限配置完成"
}

# 配置 SSH
configure_ssh() {
    log_info "配置 SSH..."
    
    local user_home="/home/$DEPLOY_USER"
    local ssh_dir="$user_home/.ssh"
    
    # 创建 .ssh 目录
    mkdir -p "$ssh_dir"
    
    # 如果提供了 SSH 公钥路径
    if [[ -n "$SSH_KEY_PATH" && -f "$SSH_KEY_PATH" ]]; then
        cp "$SSH_KEY_PATH" "$ssh_dir/authorized_keys"
        log_success "SSH 公钥已添加"
    else
        log_warning "未提供 SSH 公钥路径，请手动配置"
        log_info "可以使用以下命令添加公钥:"
        echo "  ssh-copy-id $DEPLOY_USER@$(hostname -I | awk '{print $1}')"
    fi
    
    # 设置权限
    chown -R "$DEPLOY_USER:$DEPLOY_USER" "$ssh_dir"
    chmod 700 "$ssh_dir"
    chmod 600 "$ssh_dir/authorized_keys" 2>/dev/null || true
    
    # 配置 SSH 服务
    local ssh_config="/etc/ssh/sshd_config"
    
    # 备份原配置
    cp "$ssh_config" "$ssh_config.backup"
    
    # 更新 SSH 配置
    sed -i 's/#PermitRootLogin yes/PermitRootLogin no/' "$ssh_config"
    sed -i 's/#PasswordAuthentication yes/PasswordAuthentication no/' "$ssh_config"
    sed -i 's/#PubkeyAuthentication yes/PubkeyAuthentication yes/' "$ssh_config"
    
    # 重启 SSH 服务
    systemctl restart sshd
    
    log_success "SSH 配置完成"
}

## 安装 Docker
#install_docker() {
#    log_info "安装 Docker..."
#
#    # 切换到部署用户执行安装
#    sudo -u "$DEPLOY_USER" bash << 'EOF'
#        cd /home/$DEPLOY_USER
#
#        # 下载并运行 Docker 安装脚本
#        if [[ -f "install-docker-opencloudos.sh" ]]; then
#            chmod +x /scripts/install-docker-opencloudos.sh
#            /scripts/install-docker-opencloudos.sh
#        else
#            # 使用在线安装脚本
#            curl -fsSL https://get.docker.com -o get-docker.sh
#            sh get-docker.sh
#
#            # 安装 Docker Compose
#            sudo curl -L "https://github.com/docker/compose/releases/latest/download/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
#            sudo chmod +x /usr/local/bin/docker-compose
#        fi
#EOF
#
#    # 将用户添加到 docker 组
#    usermod -aG docker "$DEPLOY_USER"
#
#    log_success "Docker 安装完成"
#}

# 配置防火墙
configure_firewall() {
    log_info "配置防火墙..."
    
    # 检查防火墙服务
    if systemctl is-active firewalld &>/dev/null; then
        # 开放必要端口
        firewall-cmd --permanent --add-port=22/tcp      # SSH
        firewall-cmd --permanent --add-port=80/tcp      # HTTP
        firewall-cmd --permanent --add-port=443/tcp     # HTTPS
        firewall-cmd --permanent --add-port=8080/tcp    # API Gateway
        firewall-cmd --permanent --add-port=8085/tcp    # WebSocket
        
        # 重载防火墙规则
        firewall-cmd --reload
        
        log_success "防火墙配置完成"
    elif command -v ufw &> /dev/null; then
        # Ubuntu UFW
        ufw allow 22/tcp
        ufw allow 80/tcp
        ufw allow 443/tcp
        ufw allow 8080/tcp
        ufw allow 8085/tcp
        ufw --force enable
        
        log_success "UFW 防火墙配置完成"
    else
        log_warning "未检测到防火墙服务，请手动配置"
    fi
}

# 克隆项目
clone_project() {
    log_info "克隆项目代码..."
    
    local user_home="/home/$DEPLOY_USER"
    local project_path="$user_home/$PROJECT_NAME"
    
    # 切换到部署用户
    sudo -u "$DEPLOY_USER" bash << EOF
        cd "$user_home"
        
        # 如果项目目录已存在，先备份
        if [[ -d "$PROJECT_NAME" ]]; then
            mv "$PROJECT_NAME" "${PROJECT_NAME}.backup.$(date +%Y%m%d-%H%M%S)"
        fi
        
        # 克隆项目
        git clone "$PROJECT_REPO" "$PROJECT_NAME"
        cd "$PROJECT_NAME"
        
        # 创建必要目录
        mkdir -p data logs config
        
        # 复制环境变量文件
        if [[ -f ".env.example" ]]; then
            cp ".env.example" ".env"
        fi
EOF
    
    log_success "项目克隆完成"
}

# 配置系统服务
configure_system_services() {
    log_info "配置系统服务..."
    
    # 创建 systemd 服务文件
    cat > "/etc/systemd/system/rustim.service" << EOF
[Unit]
Description=RustIM Instant Messaging System
Requires=docker.service
After=docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=/home/$DEPLOY_USER/$PROJECT_NAME
ExecStart=/usr/local/bin/docker-compose -f docker-compose.prod.yml up -d
ExecStop=/usr/local/bin/docker-compose -f docker-compose.prod.yml down
User=$DEPLOY_USER
Group=$DEPLOY_USER

[Install]
WantedBy=multi-user.target
EOF
    
    # 重载 systemd
    systemctl daemon-reload
    
    # 启用服务（但不立即启动）
    systemctl enable rustim.service
    
    log_success "系统服务配置完成"
}

# 配置日志轮转
configure_log_rotation() {
    log_info "配置日志轮转..."
    
    cat > "/etc/logrotate.d/rustim" << EOF
/home/$DEPLOY_USER/logs/*.log {
    daily
    missingok
    rotate 30
    compress
    delaycompress
    notifempty
    create 644 $DEPLOY_USER $DEPLOY_USER
    postrotate
        # 重启 Docker 容器以重新打开日志文件
        /usr/local/bin/docker-compose -f /home/$DEPLOY_USER/$PROJECT_NAME/docker-compose.prod.yml restart > /dev/null 2>&1 || true
    endscript
}
EOF
    
    log_success "日志轮转配置完成"
}

# 配置监控
configure_monitoring() {
    log_info "配置系统监控..."
    
    # 安装系统监控工具
    if command -v dnf &> /dev/null; then
        dnf install -y htop iotop nethogs
    elif command -v yum &> /dev/null; then
        yum install -y htop iotop nethogs
    elif command -v apt &> /dev/null; then
        apt install -y htop iotop nethogs
    fi
    
    # 创建监控脚本
    cat > "/home/$DEPLOY_USER/monitor.sh" << 'EOF'
#!/bin/bash

# RustIM 系统监控脚本

echo "=========================================="
echo "RustIM 系统状态监控"
echo "时间: $(date)"
echo "=========================================="

echo ""
echo "1. 系统资源使用情况:"
echo "CPU 使用率:"
top -bn1 | grep "Cpu(s)" | awk '{print $2}' | cut -d'%' -f1

echo ""
echo "内存使用情况:"
free -h

echo ""
echo "磁盘使用情况:"
df -h

echo ""
echo "2. Docker 容器状态:"
docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"

echo ""
echo "3. 服务端口监听状态:"
netstat -tlnp | grep -E ':(8080|8085|5432|6379|9092)'

echo ""
echo "4. 最近的错误日志:"
if [[ -f "/home/rustim/logs/error.log" ]]; then
    tail -n 10 /home/rustim/logs/error.log
else
    echo "未找到错误日志文件"
fi

echo ""
echo "=========================================="
EOF
    
    chmod +x "/home/$DEPLOY_USER/monitor.sh"
    chown "$DEPLOY_USER:$DEPLOY_USER" "/home/$DEPLOY_USER/monitor.sh"
    
    # 创建定时监控任务
    cat > "/etc/cron.d/rustim-monitor" << EOF
# RustIM 监控任务
*/5 * * * * $DEPLOY_USER /home/$DEPLOY_USER/monitor.sh >> /home/$DEPLOY_USER/logs/monitor.log 2>&1
EOF
    
    log_success "监控配置完成"
}

# 优化系统性能
optimize_system() {
    log_info "优化系统性能..."
    
    # 调整内核参数
    cat >> "/etc/sysctl.conf" << EOF

# RustIM 性能优化
net.core.somaxconn = 65535
net.core.netdev_max_backlog = 5000
net.ipv4.tcp_max_syn_backlog = 65535
net.ipv4.tcp_fin_timeout = 10
net.ipv4.tcp_keepalive_time = 1200
net.ipv4.tcp_keepalive_intvl = 15
net.ipv4.tcp_keepalive_probes = 5
net.ipv4.tcp_tw_reuse = 1
vm.swappiness = 10
vm.dirty_ratio = 15
vm.dirty_background_ratio = 5
EOF
    
    # 应用内核参数
    sysctl -p
    
    # 调整文件描述符限制
    cat >> "/etc/security/limits.conf" << EOF

# RustIM 文件描述符限制
$DEPLOY_USER soft nofile 65535
$DEPLOY_USER hard nofile 65535
root soft nofile 65535
root hard nofile 65535
EOF
    
    log_success "系统性能优化完成"
}

# 显示安装后信息
show_post_setup_info() {
    echo ""
    log_success "RustIM 服务器初始化完成！"
    echo ""
    log_info "服务器信息:"
    echo "  操作系统: $(cat /etc/os-release | grep PRETTY_NAME | cut -d'"' -f2)"
    echo "  内核版本: $(uname -r)"
    echo "  服务器 IP: $(hostname -I | awk '{print $1}')"
    echo "  部署用户: $DEPLOY_USER"
    echo "  项目路径: /home/$DEPLOY_USER/$PROJECT_NAME"
    echo ""
    log_info "已安装的服务:"
    echo "  Docker: $(docker --version 2>/dev/null || echo '未安装')"
    echo "  Docker Compose: $(docker-compose --version 2>/dev/null || echo '未安装')"
    echo "  Git: $(git --version)"
    echo ""
    log_info "下一步操作:"
    echo "  1. 配置 .env 文件: vi /home/$DEPLOY_USER/$PROJECT_NAME/.env"
    echo "  2. 构建 Docker 镜像: cd /home/$DEPLOY_USER/$PROJECT_NAME && ./scripts/deploy.sh"
    echo "  3. 启动服务: sudo systemctl start rustim"
    echo "  4. 检查状态: sudo systemctl status rustim"
    echo ""
    log_info "监控和管理:"
    echo "  系统监控: /home/$DEPLOY_USER/monitor.sh"
    echo "  服务日志: journalctl -u rustim -f"
    echo "  Docker 日志: docker-compose -f /home/$DEPLOY_USER/$PROJECT_NAME/docker-compose.prod.yml logs -f"
    echo ""
    log_warning "安全提醒:"
    echo "  1. 已禁用 root SSH 登录"
    echo "  2. 已禁用密码认证，仅允许密钥认证"
    echo "  3. 请确保 SSH 密钥安全"
    echo "  4. 定期更新系统和 Docker 镜像"
    echo ""
}

# 显示帮助信息
show_help() {
    cat << EOF
RustIM 服务器初始化脚本

用法: $0 [选项]

选项:
    -h, --help              显示帮助信息
    -u, --user USER         设置部署用户名 (默认: rustim)
    -r, --repo REPO         设置项目仓库地址
    -k, --ssh-key PATH      SSH 公钥文件路径
    --skip-docker           跳过 Docker 安装
    --skip-firewall         跳过防火墙配置
    --skip-clone            跳过项目克隆

环境变量:
    DEPLOY_USER             部署用户名
    PROJECT_REPO            项目仓库地址
    SSH_KEY_PATH            SSH 公钥路径

示例:
    $0                                          # 标准初始化
    $0 -u myuser -r https://github.com/my/repo # 自定义用户和仓库
    $0 -k ~/.ssh/id_rsa.pub                    # 指定 SSH 公钥

EOF
}

# 主函数
main() {
    local skip_docker=false
    local skip_firewall=false
    local skip_clone=false
    
    # 解析命令行参数
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                show_help
                exit 0
                ;;
            -u|--user)
                DEPLOY_USER="$2"
                shift 2
                ;;
            -r|--repo)
                PROJECT_REPO="$2"
                shift 2
                ;;
            -k|--ssh-key)
                SSH_KEY_PATH="$2"
                shift 2
                ;;
            --skip-docker)
                skip_docker=true
                shift
                ;;
            --skip-firewall)
                skip_firewall=true
                shift
                ;;
            --skip-clone)
                skip_clone=true
                shift
                ;;
            *)
                log_error "未知参数: $1"
                show_help
                exit 1
                ;;
        esac
    done
    
    echo "========================================"
    echo "    RustIM 服务器初始化脚本"
    echo "========================================"
    echo ""
    
    # 检查系统
    check_system
    
    # 更新系统
    update_system
    
    # 创建部署用户
    create_deploy_user
    
    # 配置 SSH
    configure_ssh
    
    # 安装 Docker
#    if [[ "$skip_docker" != true ]]; then
#        install_docker
#    fi
#
    # 配置防火墙
    if [[ "$skip_firewall" != true ]]; then
        configure_firewall
    fi
    
    # 克隆项目
    if [[ "$skip_clone" != true ]]; then
        clone_project
    fi
    
    # 配置系统服务
    configure_system_services
    
    # 配置日志轮转
    configure_log_rotation
    
    # 配置监控
    configure_monitoring
    
    # 优化系统性能
    optimize_system
    
    # 显示安装后信息
    show_post_setup_info
}

# 执行主函数
main "$@" 