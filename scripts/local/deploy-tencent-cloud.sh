#!/bin/bash

# RustIM 腾讯云服务器部署脚本
# 适用于已有Docker部署的数据库和Consul的环境
# 直接部署Rust二进制文件，不使用Docker和K8s
# 
# 更新日志：
# 2025-05-25: 添加 CONFIG_PATH 环境变量，确保服务能正确读取 config.yaml 配置文件

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 默认配置
ENVIRONMENT="production"
PROJECT_DIR="/home/$(whoami)/rust-im"
DEPLOY_DIR="/opt/rustim"
SERVICE_USER="rustim"
BACKUP_DIR="/home/$(whoami)/backups"
LOG_DIR="/var/log/rustim"
CONFIG_DIR="/etc/rustim"
SYSTEMD_DIR="/etc/systemd/system"

# Git 配置
GIT_REPO="git@gitee.com:chrisvip/rust-im.git"  # 改为SSH方式
GIT_BRANCH="master-feature"
# 添加SSH配置选项
USE_SSH="true"  # 默认使用SSH
SSH_KEY_PATH="$HOME/.ssh/id_rsa"  # SSH密钥路径

# 服务配置
declare -A SERVICES=(
    ["api-gateway"]="8080"
   ["msg-gateway"]="8085"
    ["user-service"]="50001"
    ["friend-service"]="50002"
    ["group-service"]="50003"
   ["msg-server"]="50004"
#   ["msg-storage"]="50005"
)

# 构建配置
RUST_VERSION="1.85"
BUILD_PROFILE="release"
CARGO_TARGET_DIR="target"

# 显示帮助信息
show_help() {
    echo "RustIM 腾讯云服务器部署脚本"
    echo ""
    echo "用法: $0 [选项] [操作] [服务名]"
    echo ""
    echo "操作:"
    echo "  deploy          完整部署 (默认)"
    echo "  build           仅构建项目"
    echo "  install         仅安装服务"
    echo "  start           启动服务 (所有服务或指定服务)"
    echo "  stop            停止服务 (所有服务或指定服务)"
    echo "  restart         重启服务 (所有服务或指定服务)"
    echo "  status          查看服务状态 (所有服务或指定服务)"
    echo "  logs            查看服务日志 (需要指定服务名)"
    echo "  backup          创建备份"
    echo "  rollback        回滚到上一版本"
    echo "  cleanup         清理旧版本"
    echo ""
    echo "服务名称:"
    echo "  api-gateway     API网关服务"
    echo "  msg-gateway     消息网关服务"
    echo "  user-service    用户服务"
    echo "  friend-service  好友服务"
    echo "  group-service   群组服务"
    echo "  msg-server      消息服务器"
    echo ""
    echo "选项:"
    echo "  -e, --environment ENV    部署环境 (staging|production) [默认: production]"
    echo "  -d, --directory DIR      项目目录 [默认: $PROJECT_DIR]"
    echo "  --deploy-dir DIR         部署目录 [默认: $DEPLOY_DIR]"
    echo "  --service-user USER      服务用户 [默认: $SERVICE_USER]"
    echo "  --build-profile PROFILE  构建配置 (debug|release) [默认: $BUILD_PROFILE]"
    echo "  --git-repo URL           Git 仓库地址 [默认: $GIT_REPO]"
    echo "  --git-branch BRANCH      Git 分支 [默认: $GIT_BRANCH]"
    echo "  --use-ssh                使用SSH方式克隆Git仓库 (推荐)"
    echo "  --use-https              使用HTTPS方式克隆Git仓库"
    echo "  --ssh-key PATH           SSH密钥路径 [默认: $SSH_KEY_PATH]"
    echo "  -h, --help              显示此帮助信息"
    echo ""
    echo "Git认证说明:"
    echo "  SSH方式 (推荐): 配置一次SSH密钥后无需再输入密码"
    echo "  HTTPS方式: 需要输入用户名密码，可通过环境变量GITEE_USERNAME和GITEE_PASSWORD设置"
    echo ""
    echo "示例:"
    echo "  $0 deploy                        # 完整部署 (默认使用SSH)"
    echo "  $0 --use-https deploy            # 使用HTTPS方式部署"
    echo "  $0 -e staging deploy             # 部署到staging环境"
    echo "  $0 build                         # 仅构建项目"
    echo "  $0 restart                       # 重启所有服务"
    echo "  $0 restart msg-gateway           # 重启消息网关服务"
    echo "  $0 start api-gateway             # 启动API网关服务"
    echo "  $0 stop user-service             # 停止用户服务"
    echo "  $0 status msg-gateway            # 查看消息网关服务状态"
    echo "  $0 logs api-gateway              # 查看API网关日志"
}

# 解析命令行参数
ACTION="deploy"
SERVICE_NAME=""

while [[ $# -gt 0 ]]; do
    case $1 in
        -e|--environment)
            ENVIRONMENT="$2"
            shift 2
            ;;
        -d|--directory)
            PROJECT_DIR="$2"
            shift 2
            ;;
        --deploy-dir)
            DEPLOY_DIR="$2"
            shift 2
            ;;
        --service-user)
            SERVICE_USER="$2"
            shift 2
            ;;
        --build-profile)
            BUILD_PROFILE="$2"
            shift 2
            ;;
        --git-repo)
            GIT_REPO="$2"
            shift 2
            ;;
        --git-branch)
            GIT_BRANCH="$2"
            shift 2
            ;;
        --use-ssh)
            USE_SSH="true"
            shift
            ;;
        --use-https)
            USE_SSH="false"
            shift
            ;;
        --ssh-key)
            SSH_KEY_PATH="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        deploy|build|install|start|stop|restart|status|logs|backup|rollback|cleanup)
            ACTION="$1"
            shift
            # 检查是否有服务名参数
            if [[ $# -gt 0 && "$1" != -* ]]; then
                SERVICE_NAME="$1"
                shift
            fi
            ;;
        *)
            # 如果前面已经有了操作，这个参数应该是服务名
            if [[ -n "$ACTION" && -z "$SERVICE_NAME" && "$1" != -* ]]; then
                SERVICE_NAME="$1"
                shift
            else
                echo "未知选项: $1"
                show_help
                exit 1
            fi
            ;;
    esac
done

# 验证环境参数
if [[ "$ENVIRONMENT" != "staging" && "$ENVIRONMENT" != "production" ]]; then
    echo -e "${RED}错误: 环境必须是 'staging' 或 'production'${NC}"
    exit 1
fi

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

# 显示部署配置信息
show_deploy_config() {
    log_info "部署配置信息:"
    echo "  - 环境: $ENVIRONMENT"
    echo "  - 项目目录: $PROJECT_DIR"
    echo "  - 部署目录: $DEPLOY_DIR"
    echo "  - 服务用户: $SERVICE_USER"
    echo "  - 构建配置: $BUILD_PROFILE"
    echo "  - Git 仓库: $GIT_REPO"
    echo "  - Git 分支: $GIT_BRANCH"
    echo "  - Git 认证方式: $(if [[ "$USE_SSH" == "true" ]]; then echo "SSH"; else echo "HTTPS"; fi)"
    if [[ "$USE_SSH" == "true" ]]; then
        echo "  - SSH 密钥路径: $SSH_KEY_PATH"
    fi
    echo "  - 日志目录: $LOG_DIR"
    echo "  - 配置目录: $CONFIG_DIR"
    echo ""
}

# 检查系统要求
check_requirements() {
    log_info "检查系统要求..."
    
    # 检查操作系统
    if [[ ! -f /etc/os-release ]]; then
        log_error "无法检测操作系统版本"
        exit 1
    fi
    
    # 检查是否为root用户或有sudo权限
    if [[ $EUID -ne 0 ]] && ! sudo -n true 2>/dev/null; then
        log_error "需要root权限或sudo权限来安装系统服务"
        log_info "请使用 sudo 运行此脚本或切换到root用户"
        exit 1
    fi
    
    # 尝试加载已安装的 Rust 环境
    if [[ -f ~/.cargo/env ]]; then
        source ~/.cargo/env
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    
    # 检查Rust环境
    if ! command -v rustc &> /dev/null; then
        log_warning "Rust 未安装，正在安装..."
        install_rust
    else
        local rust_version=$(rustc --version | awk '{print $2}')
        log_info "检测到 Rust 版本: $rust_version"
    fi
    
    # 检查Cargo
    if ! command -v cargo &> /dev/null; then
        log_error "Cargo 未安装"
        exit 1
    fi
    
    # 检查Git
    if ! command -v git &> /dev/null; then
        log_warning "Git 未安装，正在安装..."
        install_git
    else
        local git_version=$(git --version | awk '{print $3}')
        log_info "检测到 Git 版本: $git_version"
    fi
    
    log_success "系统要求检查通过"
}

# 安装Rust环境
install_rust() {
    log_info "安装 Rust 环境..."
    
    # 下载并安装rustup（使用官方源）
    log_info "下载 rustup 安装脚本..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain $RUST_VERSION
    
    # 加载Rust环境到当前shell
    if [[ -f ~/.cargo/env ]]; then
        source ~/.cargo/env
    fi
    
    # 将Rust环境变量添加到系统PATH
    if [[ -d ~/.cargo/bin ]]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    
    # 配置 Cargo 源
    configure_cargo_registry
    
    # 验证安装
    if command -v rustc &> /dev/null; then
        log_success "Rust 安装成功: $(rustc --version)"
        log_info "Cargo 版本: $(cargo --version)"
    else
        log_error "Rust 安装失败，尝试手动加载环境..."
        # 尝试从常见位置加载Rust环境
        for rust_env in ~/.cargo/env /root/.cargo/env; do
            if [[ -f "$rust_env" ]]; then
                log_info "加载 Rust 环境: $rust_env"
                source "$rust_env"
                export PATH="$HOME/.cargo/bin:$PATH"
                break
            fi
        done
        
        # 再次验证
        if command -v rustc &> /dev/null; then
            log_success "Rust 环境加载成功: $(rustc --version)"
        else
            log_error "Rust 安装失败，请检查安装过程"
            exit 1
        fi
    fi
}

# 安装Git
install_git() {
    log_info "安装 Git..."
    
    # 安装Git
    if command -v apt-get &> /dev/null; then
        # Ubuntu/Debian
        sudo apt-get update
        sudo apt-get install -y git
    elif command -v yum &> /dev/null; then
        # CentOS/RHEL
        sudo yum install -y git
    elif command -v dnf &> /dev/null; then
        # Fedora
        sudo dnf install -y git
    else
        log_warning "无法检测包管理器，请手动安装Git"
    fi
    
    log_success "Git 安装完成"
}

# 克隆或更新项目
clone_or_update_project() {
    log_info "准备项目代码..."
    
    # 配置Git凭据
    configure_git_credentials
    
    local parent_dir=$(dirname "$PROJECT_DIR")
    local project_name=$(basename "$PROJECT_DIR")
    
    # 确保父目录存在
    mkdir -p "$parent_dir"
    cd "$parent_dir"
    
    if [[ -d "$PROJECT_DIR" ]]; then
        log_info "项目目录已存在，更新代码..."
        cd "$PROJECT_DIR"
        
        # 检查是否为 Git 仓库
        if [[ -d ".git" ]]; then
            # 获取远程仓库 URL
            local current_remote=$(git remote get-url origin 2>/dev/null || echo "")
            
            if [[ "$current_remote" != "$GIT_REPO" ]]; then
                log_warning "远程仓库地址不匹配，重新克隆..."
                cd "$parent_dir"
                rm -rf "$PROJECT_DIR"
                git clone "$GIT_REPO" "$project_name"
                cd "$PROJECT_DIR"
            else
                # 拉取最新代码
                log_info "拉取最新代码..."
                git fetch origin
            fi
            
            # 切换到指定分支
            log_info "切换到分支: $GIT_BRANCH"
            if git show-ref --verify --quiet "refs/heads/$GIT_BRANCH"; then
                # 本地分支存在
                git checkout "$GIT_BRANCH"
                git pull origin "$GIT_BRANCH"
            elif git show-ref --verify --quiet "refs/remotes/origin/$GIT_BRANCH"; then
                # 远程分支存在，创建本地分支
                git checkout -b "$GIT_BRANCH" "origin/$GIT_BRANCH"
            else
                log_error "分支 $GIT_BRANCH 不存在"
                exit 1
            fi
        else
            log_warning "目录存在但不是 Git 仓库，重新克隆..."
            cd "$parent_dir"
            rm -rf "$PROJECT_DIR"
            git clone "$GIT_REPO" "$project_name"
            cd "$PROJECT_DIR"
            git checkout "$GIT_BRANCH"
        fi
    else
        log_info "克隆项目: $GIT_REPO"
        git clone "$GIT_REPO" "$project_name"
        cd "$PROJECT_DIR"
        
        # 切换到指定分支
        log_info "切换到分支: $GIT_BRANCH"
        if git show-ref --verify --quiet "refs/remotes/origin/$GIT_BRANCH"; then
            git checkout -b "$GIT_BRANCH" "origin/$GIT_BRANCH"
        else
            log_error "分支 $GIT_BRANCH 不存在"
            exit 1
        fi
    fi
    
    # 显示当前状态
    local current_branch=$(git branch --show-current)
    local current_commit=$(git rev-parse --short HEAD)
    log_success "项目准备完成"
    log_info "当前分支: $current_branch"
    log_info "当前提交: $current_commit"
}

# 创建系统用户和目录
setup_system() {
    log_info "设置系统环境..."
    
    # 创建服务用户
    if ! id "$SERVICE_USER" &>/dev/null; then
        log_info "创建服务用户: $SERVICE_USER"
        sudo useradd --system --shell /bin/false --home-dir "$DEPLOY_DIR" --create-home "$SERVICE_USER"
    fi
    
    # 创建必要目录
    log_info "创建系统目录..."
    sudo mkdir -p "$DEPLOY_DIR"/{bin,config,data,logs}
    sudo mkdir -p "$LOG_DIR"
    sudo mkdir -p "$CONFIG_DIR"
    sudo mkdir -p "$BACKUP_DIR"
    
    # 设置目录权限
    sudo chown -R "$SERVICE_USER:$SERVICE_USER" "$DEPLOY_DIR"
    sudo chown -R "$SERVICE_USER:$SERVICE_USER" "$LOG_DIR"
    sudo chmod 755 "$DEPLOY_DIR"
    sudo chmod 755 "$LOG_DIR"
    sudo chmod 755 "$CONFIG_DIR"
    
    log_success "系统环境设置完成"
}

# 安装系统依赖
install_dependencies() {
    log_info "安装系统依赖..."
    
    # 检测包管理器
    if command -v apt-get &> /dev/null; then
        # Ubuntu/Debian
        sudo apt-get update
        sudo apt-get install -y build-essential pkg-config libssl-dev libpq-dev curl wget protobuf-compiler
    elif command -v yum &> /dev/null; then
        # CentOS/RHEL
        sudo yum groupinstall -y "Development Tools"
        sudo yum install -y openssl-devel postgresql-devel curl wget
        
        # CentOS/RHEL 上通常没有现成的 protobuf-compiler 包，需要手动安装
        if ! command -v protoc &> /dev/null; then
            log_info "正在安装 Protocol Buffers 编译器..."
            
            # 创建临时目录
            local temp_dir=$(mktemp -d)
            cd "$temp_dir"
            
            # 下载 protobuf
            local protobuf_version="3.20.3"  # 选择一个稳定版本
            log_info "下载 Protocol Buffers v${protobuf_version}..."
            
            # 尝试使用国内镜像，如果失败则使用官方GitHub源
            if ! wget "https://ghproxy.com/https://github.com/protocolbuffers/protobuf/releases/download/v${protobuf_version}/protobuf-${protobuf_version}.tar.gz"; then
                log_info "使用GitHub镜像下载失败，尝试从官方源下载..."
                if ! wget "https://github.com/protocolbuffers/protobuf/releases/download/v${protobuf_version}/protobuf-${protobuf_version}.tar.gz"; then
                    # 如果仍然失败，尝试使用预编译的二进制文件
                    log_info "源码下载失败，尝试下载预编译的二进制文件..."
                    
                    # 确定系统架构
                    local arch=$(uname -m)
                    local binary_url=""
                    
                    if [[ "$arch" == "x86_64" ]]; then
                        binary_url="https://github.com/protocolbuffers/protobuf/releases/download/v${protobuf_version}/protoc-${protobuf_version}-linux-x86_64.zip"
                    elif [[ "$arch" == "aarch64" ]]; then
                        binary_url="https://github.com/protocolbuffers/protobuf/releases/download/v${protobuf_version}/protoc-${protobuf_version}-linux-aarch_64.zip"
                    else
                        log_error "不支持的系统架构: $arch"
                        exit 1
                    fi
                    
                    # 下载预编译二进制文件
                    if ! wget "$binary_url" -O "protoc.zip"; then
                        log_error "无法下载 Protocol Buffers 编译器"
                        exit 1
                    fi
                    
                    # 安装预编译二进制文件
                    sudo unzip -o "protoc.zip" -d /usr/local
                    sudo chmod 755 /usr/local/bin/protoc
                    sudo chmod -R 755 /usr/local/include/google
                    
                    # 验证安装
                    if command -v protoc &> /dev/null; then
                        log_success "Protocol Buffers 编译器安装成功: $(protoc --version)"
                    else
                        log_error "Protocol Buffers 编译器安装失败"
                        exit 1
                    fi
                    
                    # 清理并返回
                    cd /
                    rm -rf "$temp_dir"
                    return 0
                fi
            fi
            
            # 解压
            tar -xzf "protobuf-${protobuf_version}.tar.gz"
            cd "protobuf-${protobuf_version}"
            
            # 配置、编译和安装
            log_info "编译和安装 Protocol Buffers..."
            ./configure
            make -j$(nproc)
            sudo make install
            sudo ldconfig
            
            # 清理
            cd /
            rm -rf "$temp_dir"
            
            # 验证安装
            if command -v protoc &> /dev/null; then
                log_success "Protocol Buffers 编译器安装成功: $(protoc --version)"
            else
                log_error "Protocol Buffers 编译器安装失败"
                exit 1
            fi
        fi
    elif command -v dnf &> /dev/null; then
        # Fedora
        sudo dnf groupinstall -y "Development Tools"
        sudo dnf install -y openssl-devel postgresql-devel curl wget protobuf-compiler
    else
        log_warning "无法检测包管理器，请手动安装构建依赖"
        log_warning "请确保已安装 Protocol Buffers 编译器 (protoc)"
    fi
    
    # 验证 protoc 是否已安装
    if command -v protoc &> /dev/null; then
        log_success "检测到 Protocol Buffers 编译器: $(protoc --version)"
    else
        log_error "未找到 Protocol Buffers 编译器，请手动安装"
        log_info "可以访问 https://github.com/protocolbuffers/protobuf/releases 下载"
        exit 1
    fi
    
    log_success "系统依赖安装完成"
}

# 构建项目
build_project() {
    log_info "构建项目..."
    
    cd "$PROJECT_DIR"
    
    # 确保 Rust 环境已加载
    if [[ -f ~/.cargo/env ]]; then
        source ~/.cargo/env
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
    
    # 验证 Rust 环境
    if ! command -v rustc &> /dev/null || ! command -v cargo &> /dev/null; then
        log_error "Rust 环境未正确加载，请检查安装"
        log_info "当前 PATH: $PATH"
        exit 1
    fi
    
    log_info "使用 Rust 版本: $(rustc --version)"
    log_info "使用 Cargo 版本: $(cargo --version)"
    
    # 设置Rust环境变量
    export RUST_BACKTRACE=1
    export CARGO_TARGET_DIR="$CARGO_TARGET_DIR"
    export CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_DEBUG=true
    
    # 如果 protoc 在默认路径不可用，设置 PROTOC 环境变量
    if ! command -v protoc &> /dev/null; then
        # 检查是否有自定义安装的 protoc
        for protoc_path in /usr/local/bin/protoc /usr/bin/protoc; do
            if [[ -x "$protoc_path" ]]; then
                log_info "设置 PROTOC 环境变量为: $protoc_path"
                export PROTOC="$protoc_path"
                break
            fi
        done
    fi
    
    # 配置 Cargo 源
    configure_cargo_registry
    
    # 清理之前的构建
    if [[ "$BUILD_PROFILE" == "release" ]]; then
        log_info "清理之前的构建..."
        cargo clean --release
    fi
    
    # 更新依赖
    log_info "更新项目依赖..."
    cargo update
    
    # 构建所有服务
    log_info "构建所有服务 (配置: $BUILD_PROFILE)..."
    log_info "使用并行构建以加速编译过程..."
    
    if [[ "$BUILD_PROFILE" == "release" ]]; then
        log_info "执行: cargo build --release --workspace --jobs $(nproc)"
        cargo build --release --workspace --jobs $(nproc)
    else
        log_info "执行: cargo build --workspace --jobs $(nproc)"
        cargo build --workspace --jobs $(nproc)
    fi
    
    # 验证构建结果
    local build_dir="$CARGO_TARGET_DIR/$BUILD_PROFILE"
    for service in "${!SERVICES[@]}"; do
        local binary_path="$build_dir/$service"
        if [[ ! -f "$binary_path" ]]; then
            log_error "服务 $service 构建失败: $binary_path 不存在"
            exit 1
        fi
        local file_size=$(du -h "$binary_path" | cut -f1)
        log_success "服务 $service 构建成功 (大小: $file_size)"
    done
    
    log_success "项目构建完成"
}

# 配置 Cargo 源
configure_cargo_registry() {
    log_info "配置 Cargo 源..."
    
    # 清理现有配置和缓存
    rm -f ~/.cargo/config
    rm -f ~/.cargo/config.toml
    
    # 清理 git 数据库和索引缓存
    if [[ -d ~/.cargo/registry/index ]]; then
        log_info "清理 Cargo 索引缓存..."
        rm -rf ~/.cargo/registry/index/* || log_warning "清理 Cargo 索引缓存失败，这可能是由于权限问题"
    fi
    
    if [[ -d ~/.cargo/git/db ]]; then
        log_info "清理 Cargo 仓库缓存..."
        rm -rf ~/.cargo/git/db/* || log_warning "清理 Cargo 仓库缓存失败，这可能是由于权限问题"
    fi
    
    # 定义可能的源
    local sources=(
        "https://github.com/rust-lang/crates.io-index"  # 官方源
        "https://mirrors.tuna.tsinghua.edu.cn/git/crates.io-index.git"  # 清华源
        "https://mirrors.ustc.edu.cn/crates.io-index"  # 中科大源
        "https://rsproxy.cn/crates.io-index"  # RsProxy源
    )
    
    # 检查网络连接
    log_info "测试各镜像源网络连接..."
    local selected_source=""
    
    for source in "${sources[@]}"; do
        log_info "测试源: $source"
        local domain=$(echo "$source" | sed -E 's|https?://([^/]+)/.*|\1|')
        
        # 测试连接
        if curl --connect-timeout 5 -s -o /dev/null -w "%{http_code}" "https://$domain" | grep -q "2[0-9][0-9]\|3[0-9][0-9]"; then
            log_success "源 $source 连接成功"
            selected_source="$source"
            break
        else
            log_warning "源 $source 连接失败，尝试下一个"
        fi
    done
    
    # 如果所有源都连接失败，使用官方源
    if [[ -z "$selected_source" ]]; then
        log_warning "所有镜像源测试失败，使用官方源"
        selected_source="${sources[0]}"
    fi
    
    # 创建 Cargo 配置
    mkdir -p ~/.cargo
    log_info "使用 Cargo 源: $selected_source"
    
    cat > ~/.cargo/config << EOF
[source.crates-io]
registry = "$selected_source"

[net]
git-fetch-with-cli = true
EOF
    
    log_success "Cargo 源配置完成"
}

# 安装服务二进制文件
install_binaries() {
    log_info "安装服务二进制文件..."
    
    cd "$PROJECT_DIR"
    local build_dir="$CARGO_TARGET_DIR/$BUILD_PROFILE"
    
    # 创建版本目录
    local timestamp=$(date +"%Y%m%d_%H%M%S")
    local version_dir="$DEPLOY_DIR/versions/$timestamp"
    sudo mkdir -p "$version_dir"
    
    # 复制二进制文件
    for service in "${!SERVICES[@]}"; do
        local binary_path="$build_dir/$service"
        if [[ -f "$binary_path" ]]; then
            log_info "安装 $service..."
            sudo cp "$binary_path" "$version_dir/"
            sudo chmod +x "$version_dir/$service"
        else
            log_error "二进制文件不存在: $binary_path"
            exit 1
        fi
    done
    
    # 创建符号链接到当前版本
    sudo rm -f "$DEPLOY_DIR/bin"/*
    for service in "${!SERVICES[@]}"; do
        sudo ln -sf "$version_dir/$service" "$DEPLOY_DIR/bin/$service"
    done
    
    # 设置权限
    sudo chown -R "$SERVICE_USER:$SERVICE_USER" "$version_dir"
    sudo chown -R "$SERVICE_USER:$SERVICE_USER" "$DEPLOY_DIR/bin"
    
    # 记录当前版本
    echo "$timestamp" | sudo tee "$DEPLOY_DIR/current_version" > /dev/null
    
    log_success "服务二进制文件安装完成"
}

# 配置服务
configure_services() {
    log_info "配置服务..."
    
    # 复制配置文件
    log_info "复制配置文件..."
    sudo cp -r "$PROJECT_DIR/config"/* "$CONFIG_DIR/"
    
    # 复制环境配置
    local env_file="$PROJECT_DIR/.env.$ENVIRONMENT"
    if [[ -f "$env_file" ]]; then
        sudo cp "$env_file" "$CONFIG_DIR/.env"
    else
        sudo cp "$PROJECT_DIR/.env" "$CONFIG_DIR/.env"
    fi
    
    # 设置配置文件权限
    sudo chown -R "$SERVICE_USER:$SERVICE_USER" "$CONFIG_DIR"
    sudo chmod 640 "$CONFIG_DIR/.env"
    
    # 确保config.yaml有正确的权限
    if [[ -f "$CONFIG_DIR/config.yaml" ]]; then
        log_info "设置 config.yaml 权限..."
        sudo chmod 644 "$CONFIG_DIR/config.yaml"
        sudo chown "$SERVICE_USER:$SERVICE_USER" "$CONFIG_DIR/config.yaml"
    else
        log_error "配置文件 config.yaml 不存在"
    fi
    
    log_success "服务配置完成"
}

# 创建systemd服务文件
create_systemd_services() {
    log_info "创建 systemd 服务文件..."
    
    for service in "${!SERVICES[@]}"; do
        local port="${SERVICES[$service]}"
        local service_file="$SYSTEMD_DIR/rustim-$service.service"
        
        log_info "创建 $service 服务文件..."
        
        sudo tee "$service_file" > /dev/null <<EOF
[Unit]
Description=RustIM $service Service
After=network.target
Wants=network.target

[Service]
Type=simple
User=$SERVICE_USER
Group=$SERVICE_USER
WorkingDirectory=$DEPLOY_DIR
ExecStart=$DEPLOY_DIR/bin/$service
ExecReload=/bin/kill -HUP \$MAINPID
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=rustim-$service

# 环境变量
Environment=RUST_LOG=info
Environment=RUST_BACKTRACE=1
Environment=CONFIG_PATH=$CONFIG_DIR/config.yaml
EnvironmentFile=$CONFIG_DIR/.env

# 安全设置
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$DEPLOY_DIR $LOG_DIR

# 资源限制
LimitNOFILE=65536
LimitNPROC=4096

[Install]
WantedBy=multi-user.target
EOF
    done
    
    # 重新加载systemd配置
    sudo systemctl daemon-reload
    
    log_success "systemd 服务文件创建完成"
}

# 验证服务名称
validate_service_name() {
    local service_name="$1"
    
    if [[ -z "$service_name" ]]; then
        return 0  # 空名称表示操作所有服务
    fi
    
    if [[ -z "${SERVICES[$service_name]}" ]]; then
        log_error "未知服务: $service_name"
        log_info "可用服务: ${!SERVICES[*]}"
        exit 1
    fi
    
    return 0
}

# 获取要操作的服务列表
get_services_to_operate() {
    local service_name="$1"
    
    if [[ -n "$service_name" ]]; then
        echo "$service_name"
    else
        echo "${!SERVICES[@]}"
    fi
}

# 启用并启动服务
enable_and_start_services() {
    local target_service="$1"
    validate_service_name "$target_service"
    
    local services_to_start=($(get_services_to_operate "$target_service"))
    
    if [[ -n "$target_service" ]]; then
        log_info "启用并启动服务: $target_service"
    else
        log_info "启用并启动所有服务..."
    fi
    
    # 确认配置文件存在
    if [[ -f "$CONFIG_DIR/config.yaml" ]]; then
        log_info "配置文件已准备: $CONFIG_DIR/config.yaml"
    else
        log_error "配置文件不存在: $CONFIG_DIR/config.yaml"
        log_warning "服务可能无法正常启动，请确保配置文件存在"
    fi
    
    # 记录失败的服务和成功的服务
    local failed_services=()
    local success_services=()
    
    for service in "${services_to_start[@]}"; do
        local service_name="rustim-$service"
        
        log_info "启用服务: $service_name"
        # 即使启用失败也继续
        sudo systemctl enable "$service_name" || log_warning "服务 $service_name 启用失败，但将继续尝试启动"
        
        log_info "启动服务: $service_name"
        # 尝试启动服务
        sudo systemctl start "$service_name" 
        
        # 等待服务启动
        sleep 2
        
        # 检查服务状态
        if sudo systemctl is-active --quiet "$service_name"; then
            log_success "服务 $service_name 启动成功"
            success_services+=("$service_name")
        else
            log_error "服务 $service_name 启动失败"
            failed_services+=("$service_name")
            # 显示失败服务的状态以便调试
            sudo systemctl status "$service_name" || true
            log_warning "尽管服务 $service_name 启动失败，但将继续启动其他服务"
        fi
    done
    
    # 汇总报告
    echo ""
    log_info "服务启动汇总:"
    if [[ ${#success_services[@]} -gt 0 ]]; then
        echo -e "${GREEN}成功启动的服务 (${#success_services[@]})${NC}:"
        for service in "${success_services[@]}"; do
            echo -e "  - ${GREEN}$service${NC}"
        done
    fi
    
    if [[ ${#failed_services[@]} -gt 0 ]]; then
        echo -e "${RED}启动失败的服务 (${#failed_services[@]})${NC}:"
        for service in "${failed_services[@]}"; do
            echo -e "  - ${RED}$service${NC}"
        done
        log_warning "有 ${#failed_services[@]} 个服务启动失败，请检查服务日志获取详细信息"
        log_info "可以使用 'sudo journalctl -u SERVICE_NAME' 查看特定服务的日志"
    else
        if [[ -n "$target_service" ]]; then
            log_success "服务 $target_service 启动成功"
        else
            log_success "所有服务启动成功"
        fi
    fi
}

# 停止服务
stop_services() {
    local target_service="$1"
    validate_service_name "$target_service"
    
    local services_to_stop=($(get_services_to_operate "$target_service"))
    
    if [[ -n "$target_service" ]]; then
        log_info "停止服务: $target_service"
    else
        log_info "停止所有服务..."
    fi
    
    local stopped_services=()
    local already_stopped_services=()
    
    for service in "${services_to_stop[@]}"; do
        local service_name="rustim-$service"
        
        if sudo systemctl is-active --quiet "$service_name"; then
            log_info "停止服务: $service_name"
            sudo systemctl stop "$service_name"
            stopped_services+=("$service_name")
            log_success "服务 $service_name 已停止"
        else
            log_info "服务 $service_name 已经是停止状态"
            already_stopped_services+=("$service_name")
        fi
    done
    
    # 汇总报告
    echo ""
    if [[ ${#stopped_services[@]} -gt 0 ]]; then
        log_info "已停止的服务 (${#stopped_services[@]}):"
        for service in "${stopped_services[@]}"; do
            echo -e "  - ${GREEN}$service${NC}"
        done
    fi
    
    if [[ ${#already_stopped_services[@]} -gt 0 ]]; then
        log_info "原本就是停止状态的服务 (${#already_stopped_services[@]}):"
        for service in "${already_stopped_services[@]}"; do
            echo -e "  - ${YELLOW}$service${NC}"
        done
    fi
    
    if [[ -n "$target_service" ]]; then
        log_success "服务 $target_service 操作完成"
    else
        log_success "所有服务停止操作完成"
    fi
}

# 重启服务
restart_services() {
    local target_service="$1"
    validate_service_name "$target_service"
    
    local services_to_restart=($(get_services_to_operate "$target_service"))
    
    if [[ -n "$target_service" ]]; then
        log_info "重启服务: $target_service"
    else
        log_info "重启所有服务..."
    fi
    
    # 记录失败的服务和成功的服务
    local failed_services=()
    local success_services=()
    
    for service in "${services_to_restart[@]}"; do
        local service_name="rustim-$service"
        
        log_info "重启服务: $service_name"
        sudo systemctl restart "$service_name"
        
        # 等待服务启动
        sleep 2
        
        # 检查服务状态
        if sudo systemctl is-active --quiet "$service_name"; then
            log_success "服务 $service_name 重启成功"
            success_services+=("$service_name")
        else
            log_error "服务 $service_name 重启失败"
            failed_services+=("$service_name")
            # 显示失败服务的状态以便调试
            sudo systemctl status "$service_name" || true
            log_warning "尽管服务 $service_name 重启失败，但将继续重启其他服务"
        fi
    done
    
    # 汇总报告
    echo ""
    log_info "服务重启汇总:"
    if [[ ${#success_services[@]} -gt 0 ]]; then
        echo -e "${GREEN}成功重启的服务 (${#success_services[@]})${NC}:"
        for service in "${success_services[@]}"; do
            echo -e "  - ${GREEN}$service${NC}"
        done
    fi
    
    if [[ ${#failed_services[@]} -gt 0 ]]; then
        echo -e "${RED}重启失败的服务 (${#failed_services[@]})${NC}:"
        for service in "${failed_services[@]}"; do
            echo -e "  - ${RED}$service${NC}"
        done
        log_warning "有 ${#failed_services[@]} 个服务重启失败，请检查服务日志获取详细信息"
    else
        if [[ -n "$target_service" ]]; then
            log_success "服务 $target_service 重启成功"
        else
            log_success "所有服务重启成功"
        fi
    fi
}

# 查看服务状态
show_status() {
    local target_service="$1"
    validate_service_name "$target_service"
    
    local services_to_check=($(get_services_to_operate "$target_service"))
    
    if [[ -n "$target_service" ]]; then
        log_info "服务状态: $target_service"
    else
        log_info "所有服务状态:"
    fi
    echo ""
    
    for service in "${services_to_check[@]}"; do
        local service_name="rustim-$service"
        local port="${SERVICES[$service]}"
        
        echo -n "  $service_name (端口 $port): "
        if sudo systemctl is-active --quiet "$service_name"; then
            echo -e "${GREEN}运行中${NC}"
        else
            echo -e "${RED}已停止${NC}"
        fi
    done
    
    echo ""
    if [[ -n "$target_service" ]]; then
        log_info "详细状态: $target_service"
        local service_name="rustim-$target_service"
        echo ""
        echo "=== $service_name ==="
        sudo systemctl status "$service_name" --no-pager -l
    else
        log_info "详细状态:"
        for service in "${services_to_check[@]}"; do
            local service_name="rustim-$service"
            echo ""
            echo "=== $service_name ==="
            sudo systemctl status "$service_name" --no-pager -l
        done
    fi
}

# 查看服务日志
show_logs() {
    local service_name="$1"
    
    if [[ -n "$service_name" ]]; then
        # 查看特定服务日志
        if [[ -n "${SERVICES[$service_name]}" ]]; then
            log_info "查看 $service_name 服务日志:"
            sudo journalctl -u "rustim-$service_name" -f --no-pager
        else
            log_error "未知服务: $service_name"
            log_info "可用服务: ${!SERVICES[*]}"
            exit 1
        fi
    else
        # 查看所有服务日志
        log_info "查看所有服务日志:"
        local service_units=""
        for service in "${!SERVICES[@]}"; do
            service_units="$service_units -u rustim-$service"
        done
        sudo journalctl $service_units -f --no-pager
    fi
}

# 创建备份
create_backup() {
    log_info "创建备份..."
    
    local timestamp=$(date +"%Y%m%d_%H%M%S")
    local backup_name="rustim_${ENVIRONMENT}_${timestamp}"
    local backup_path="$BACKUP_DIR/$backup_name"
    
    # 创建备份目录
    mkdir -p "$backup_path"
    
    # 备份当前版本信息
    if [[ -f "$DEPLOY_DIR/current_version" ]]; then
        cp "$DEPLOY_DIR/current_version" "$backup_path/"
    fi
    
    # 备份配置文件
    cp -r "$CONFIG_DIR" "$backup_path/"
    
    # 备份二进制文件
    if [[ -d "$DEPLOY_DIR/bin" ]]; then
        cp -r "$DEPLOY_DIR/bin" "$backup_path/"
    fi
    
    # 备份systemd服务文件
    mkdir -p "$backup_path/systemd"
    for service in "${!SERVICES[@]}"; do
        local service_file="$SYSTEMD_DIR/rustim-$service.service"
        if [[ -f "$service_file" ]]; then
            cp "$service_file" "$backup_path/systemd/"
        fi
    done
    
    # 清理旧备份（保留最近 5 个）
    cd "$BACKUP_DIR"
    ls -t | grep "rustim_${ENVIRONMENT}_" | tail -n +6 | xargs -r rm -rf
    
    log_success "备份创建完成: $backup_path"
}

# 回滚到上一版本
rollback() {
    log_info "回滚到上一版本..."
    
    # 查找上一个版本
    local versions_dir="$DEPLOY_DIR/versions"
    if [[ ! -d "$versions_dir" ]]; then
        log_error "版本目录不存在: $versions_dir"
        exit 1
    fi
    
    local current_version=""
    if [[ -f "$DEPLOY_DIR/current_version" ]]; then
        current_version=$(cat "$DEPLOY_DIR/current_version")
    fi
    
    # 获取所有版本，按时间排序
    local versions=($(ls -t "$versions_dir"))
    local previous_version=""
    
    for version in "${versions[@]}"; do
        if [[ "$version" != "$current_version" ]]; then
            previous_version="$version"
            break
        fi
    done
    
    if [[ -z "$previous_version" ]]; then
        log_error "未找到可回滚的版本"
        exit 1
    fi
    
    log_info "回滚到版本: $previous_version"
    
    # 停止服务
    stop_services
    
    # 更新符号链接
    sudo rm -f "$DEPLOY_DIR/bin"/*
    for service in "${!SERVICES[@]}"; do
        sudo ln -sf "$versions_dir/$previous_version/$service" "$DEPLOY_DIR/bin/$service"
    done
    
    # 更新当前版本记录
    echo "$previous_version" | sudo tee "$DEPLOY_DIR/current_version" > /dev/null
    
    # 启动服务
    enable_and_start_services
    
    log_success "回滚完成"
}

# 清理旧版本
cleanup_old_versions() {
    log_info "清理旧版本..."
    
    local versions_dir="$DEPLOY_DIR/versions"
    if [[ ! -d "$versions_dir" ]]; then
        log_warning "版本目录不存在: $versions_dir"
        return 0
    fi
    
    local current_version=""
    if [[ -f "$DEPLOY_DIR/current_version" ]]; then
        current_version=$(cat "$DEPLOY_DIR/current_version")
    fi
    
    # 保留最近 3 个版本
    cd "$versions_dir"
    local versions=($(ls -t))
    local keep_count=3
    local removed_count=0
    
    for ((i=$keep_count; i<${#versions[@]}; i++)); do
        local version="${versions[$i]}"
        if [[ "$version" != "$current_version" ]]; then
            log_info "删除旧版本: $version"
            sudo rm -rf "$version"
            ((removed_count++))
        fi
    done
    
    log_success "清理完成，删除了 $removed_count 个旧版本"
}

# 健康检查
health_check() {
    log_info "执行健康检查..."
    
    # 记录健康和不健康的服务
    local healthy_services=()
    local unhealthy_services=()
    local reasons=()
    
    for service in "${!SERVICES[@]}"; do
        local port="${SERVICES[$service]}"
        local service_name="rustim-$service"
        local is_healthy=true
        local unhealthy_reason=""
        
        # 检查服务状态
        if ! sudo systemctl is-active --quiet "$service_name"; then
            is_healthy=false
            unhealthy_reason="服务未运行"
        # 只有服务运行时才检查端口
        elif ! netstat -tuln | grep -q ":$port "; then
            is_healthy=false
            unhealthy_reason="端口 $port 未监听"
        # 检查HTTP健康端点（如果有）
        elif [[ "$service" == "api-gateway" ]]; then
            if ! curl -f -s "http://localhost:$port/health" > /dev/null; then
                is_healthy=false
                unhealthy_reason="健康检查API返回非成功状态"
            fi
        fi
        
        # 记录服务健康状态
        if [[ "$is_healthy" == "true" ]]; then
            healthy_services+=("$service_name")
            log_success "服务 $service_name 健康检查通过"
        else
            unhealthy_services+=("$service_name")
            reasons+=("$service_name: $unhealthy_reason")
            log_error "服务 $service_name 健康检查失败: $unhealthy_reason"
        fi
    done
    
    # 输出健康检查汇总
    echo ""
    log_info "健康检查汇总:"
    if [[ ${#healthy_services[@]} -gt 0 ]]; then
        echo -e "${GREEN}健康的服务 (${#healthy_services[@]})${NC}:"
        for service in "${healthy_services[@]}"; do
            echo -e "  - ${GREEN}$service${NC}"
        done
    fi
    
    if [[ ${#unhealthy_services[@]} -gt 0 ]]; then
        echo -e "${RED}不健康的服务 (${#unhealthy_services[@]})${NC}:"
        for ((i=0; i<${#unhealthy_services[@]}; i++)); do
            echo -e "  - ${RED}${reasons[$i]}${NC}"
        done
        log_warning "有 ${#unhealthy_services[@]} 个服务健康检查失败"
        # 返回失败状态码，但不中断部署流程
        return 1
    else
        log_success "所有服务健康检查通过"
        return 0
    fi
}

# 添加SSH密钥配置函数
setup_ssh_key() {
    log_info "配置SSH密钥..."
    
    # 检查SSH密钥是否存在
    if [[ ! -f "$SSH_KEY_PATH" ]]; then
        log_warning "SSH密钥不存在: $SSH_KEY_PATH"
        log_info "正在生成新的SSH密钥..."
        
        # 生成SSH密钥
        ssh-keygen -t rsa -b 4096 -C "rustim-deploy@$(hostname)" -f "$SSH_KEY_PATH" -N ""
        
        log_success "SSH密钥已生成: $SSH_KEY_PATH"
        log_warning "请将以下公钥添加到Gitee账户的SSH密钥中:"
        echo "----------------------------------------"
        cat "${SSH_KEY_PATH}.pub"
        echo "----------------------------------------"
        log_info "添加步骤:"
        log_info "1. 登录Gitee -> 设置 -> SSH公钥"
        log_info "2. 点击'添加公钥'"
        log_info "3. 复制上面的公钥内容并粘贴"
        log_info "4. 点击'确定'"
        echo ""
        read -p "请确认已添加SSH公钥到Gitee后按回车继续..."
    else
        log_info "SSH密钥已存在: $SSH_KEY_PATH"
    fi
    
    # 确保SSH密钥权限正确
    chmod 600 "$SSH_KEY_PATH"
    chmod 644 "${SSH_KEY_PATH}.pub"
    
    # 启动ssh-agent并添加密钥
    if ! pgrep -x "ssh-agent" > /dev/null; then
        eval "$(ssh-agent -s)"
    fi
    ssh-add "$SSH_KEY_PATH" 2>/dev/null || true
    
    # 测试SSH连接
    log_info "测试SSH连接到Gitee..."
    if ssh -T git@gitee.com -o StrictHostKeyChecking=no -o ConnectTimeout=10 2>&1 | grep -q "successfully authenticated"; then
        log_success "SSH连接测试成功"
    else
        log_warning "SSH连接测试失败，但将继续尝试克隆"
        log_info "如果克隆失败，请检查SSH密钥配置"
    fi
}

# 配置Git凭据存储
configure_git_credentials() {
    log_info "配置Git凭据..."
    
    if [[ "$USE_SSH" == "true" ]]; then
        setup_ssh_key
        # 确保使用SSH URL
        GIT_REPO="git@gitee.com:chrisvip/rust-im.git"
    else
        # 使用HTTPS方式，配置凭据存储
        log_info "配置Git凭据存储..."
        git config --global credential.helper store
        git config --global credential.helper 'cache --timeout=86400'  # 24小时缓存
        
        # 如果设置了环境变量，使用它们
        if [[ -n "$GITEE_USERNAME" && -n "$GITEE_PASSWORD" ]]; then
            log_info "使用环境变量中的Git凭据"
            # 创建凭据文件
            echo "https://${GITEE_USERNAME}:${GITEE_PASSWORD}@gitee.com" > ~/.git-credentials
            chmod 600 ~/.git-credentials
        else
            log_warning "未设置GITEE_USERNAME和GITEE_PASSWORD环境变量"
            log_info "请在首次克隆时输入用户名和密码，之后会自动缓存"
        fi
        
        # 确保使用HTTPS URL
        GIT_REPO="https://gitee.com/chrisvip/rust-im.git"
    fi
}

# 主函数
main() {
    echo "RustIM 腾讯云服务器部署脚本"
    echo "================================"
    echo ""
    
    show_deploy_config
    
    case "$ACTION" in
        deploy)
            check_requirements
            clone_or_update_project
            install_dependencies
            setup_system
            build_project
            create_backup
            install_binaries
            configure_services
            create_systemd_services
            enable_and_start_services
            # 执行健康检查但不中断部署
            if ! health_check; then
                log_warning "部分服务健康检查失败，但部署流程已完成"
                log_info "请检查服务日志以排除故障，然后尝试手动重启失败的服务"
                log_success "部署完成，但需要注意上述警告！"
            else
                log_success "部署完成，所有服务健康检查通过！"
            fi
            ;;
        build)
            check_requirements
            clone_or_update_project
            build_project
            ;;
        install)
            check_requirements
            setup_system
            install_binaries
            configure_services
            create_systemd_services
            ;;
        start)
            enable_and_start_services "$SERVICE_NAME"
            ;;
        stop)
            stop_services "$SERVICE_NAME"
            ;;
        restart)
            restart_services "$SERVICE_NAME"
            ;;
        status)
            show_status "$SERVICE_NAME"
            ;;
        logs)
            show_logs "$SERVICE_NAME"
            ;;
        backup)
            create_backup
            ;;
        rollback)
            rollback
            ;;
        cleanup)
            cleanup_old_versions
            ;;
        *)
            log_error "未知操作: $ACTION"
            show_help
            exit 1
            ;;
    esac
}

# 执行主函数
main "$@" 