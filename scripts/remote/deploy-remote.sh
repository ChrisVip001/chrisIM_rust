#!/bin/bash

# RustIM 远程部署脚本 - 针对腾讯云 OpenCloudOS 优化
# 支持多环境部署：staging 和 production

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 默认配置
ENVIRONMENT="staging"
PROJECT_DIR="/home/$(whoami)/rust-im"
DOCKER_COMPOSE_FILE="docker-compose.yml"
BACKUP_DIR="/home/$(whoami)/backups"

# Docker Compose 命令检测
DOCKER_COMPOSE_CMD=""

# 检测 Docker Compose 版本并设置命令
detect_docker_compose() {
    if docker compose version &> /dev/null; then
        DOCKER_COMPOSE_CMD="docker compose"
        log_info "检测到 Docker Compose V2 (Plugin)"
    elif command -v docker-compose &> /dev/null; then
        DOCKER_COMPOSE_CMD="docker-compose"
        log_info "检测到 Docker Compose V1 (独立版本)"
    else
        log_error "Docker Compose 未安装"
        log_info "请运行以下命令安装："
        log_info "  ./scripts/install-docker-compose.sh"
        exit 1
    fi
}

# 显示帮助信息
show_help() {
    echo "RustIM 远程部署脚本"
    echo ""
    echo "用法: $0 [选项]"
    echo ""
    echo "选项:"
    echo "  -e, --environment ENV    部署环境 (staging|production) [默认: production]"
    echo "  -d, --directory DIR      项目目录 [默认: $PROJECT_DIR]"
    echo "  -f, --compose-file FILE  Docker Compose 文件 [默认: $DOCKER_COMPOSE_FILE]"
    echo "  -b, --backup-dir DIR     备份目录 [默认: $BACKUP_DIR]"
    echo "  -h, --help              显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  $0 -e staging"
    echo "  $0 -e production -d /opt/rust-im"
}

# 解析命令行参数
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
        -f|--compose-file)
            DOCKER_COMPOSE_FILE="$2"
            shift 2
            ;;
        -b|--backup-dir)
            BACKUP_DIR="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "未知选项: $1"
            show_help
            exit 1
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

# 检查系统要求
check_requirements() {
    log_info "检查系统要求..."
    
    # 检查 Docker
    if ! command -v docker &> /dev/null; then
        log_error "Docker 未安装"
        exit 1
    fi
    
    # 检查 Git
    if ! command -v git &> /dev/null; then
        log_error "Git 未安装"
        exit 1
    fi
    
    # 检查项目目录
    if [[ ! -d "$PROJECT_DIR" ]]; then
        log_error "项目目录不存在: $PROJECT_DIR"
        exit 1
    fi
    
    log_success "系统要求检查通过"
}

# 创建备份
create_backup() {
    log_info "创建备份..."
    
    local timestamp=$(date +"%Y%m%d_%H%M%S")
    local backup_name="rustim_${ENVIRONMENT}_${timestamp}"
    local backup_path="$BACKUP_DIR/$backup_name"
    
    # 创建备份目录
    mkdir -p "$backup_path"
    
    # 备份配置文件
    if [[ -f "$PROJECT_DIR/.env" ]]; then
        cp "$PROJECT_DIR/.env" "$backup_path/"
    fi
    
    if [[ -f "$PROJECT_DIR/$DOCKER_COMPOSE_FILE" ]]; then
        cp "$PROJECT_DIR/$DOCKER_COMPOSE_FILE" "$backup_path/"
    fi
    
    # 备份数据库（如果运行中）
    if $DOCKER_COMPOSE_CMD -f "$PROJECT_DIR/$DOCKER_COMPOSE_FILE" ps postgres | grep -q "Up"; then
        log_info "备份数据库..."
        $DOCKER_COMPOSE_CMD -f "$PROJECT_DIR/$DOCKER_COMPOSE_FILE" exec -T postgres pg_dump -U rustim rustim > "$backup_path/database_backup.sql"
    fi
    
    # 清理旧备份（保留最近 5 个）
    cd "$BACKUP_DIR"
    ls -t | grep "rustim_${ENVIRONMENT}_" | tail -n +6 | xargs -r rm -rf
    
    log_success "备份创建完成: $backup_path"
}

# 设置环境配置
setup_environment() {
    log_info "设置 $ENVIRONMENT 环境配置..."
    
    cd "$PROJECT_DIR"
    
    # 根据环境选择配置文件
    local env_file=".env.${ENVIRONMENT}"
    if [[ -f "$env_file" ]]; then
        cp "$env_file" ".env"
        log_success "已应用 $ENVIRONMENT 环境配置"
    else
        log_warning "环境配置文件 $env_file 不存在，使用默认配置"
    fi
    
    # 根据环境选择 Docker Compose 文件
    local compose_file="docker-compose.${ENVIRONMENT}.yml"
    if [[ -f "$compose_file" ]]; then
        DOCKER_COMPOSE_FILE="$compose_file"
        log_success "使用 $ENVIRONMENT 环境的 Docker Compose 配置"
    fi
}
# 拉取最新代码
pull_latest_code() {
    log_info "拉取最新代码..."

    cd "$PROJECT_DIR"

    # 配置 Git 安全目录（解决 dubious ownership 问题）
    log_info "配置 Git 安全目录..."
    git config --global --add safe.directory "$PROJECT_DIR" 2>/dev/null || true

    # 检查是否是 Git 仓库
    if [[ ! -d ".git" ]]; then
        log_warning "当前目录不是 Git 仓库，跳过代码更新"
        return 0
    fi

    # 根据环境确定分支
    local branch
    if [[ "$ENVIRONMENT" == "staging" ]]; then
        branch="develop"
    else
        branch="release"
    fi

    # 检查当前分支
    local current_branch=$(git branch --show-current 2>/dev/null || echo "unknown")
    if [[ "$current_branch" != "$branch" ]]; then
        log_info "切换到 $branch 分支..."

        # 检查分支是否存在
        if git show-ref --verify --quiet "refs/heads/$branch" 2>/dev/null; then
            git checkout "$branch"
        elif git show-ref --verify --quiet "refs/remotes/origin/$branch" 2>/dev/null; then
            git checkout -b "$branch" "origin/$branch"
        else
            log_warning "分支 $branch 不存在，保持当前分支"
            return 0
        fi
    fi

    # 拉取最新代码
    log_info "拉取 $branch 分支的最新代码..."
    if git pull origin "$branch" 2>/dev/null; then
        log_success "代码更新完成"
    else
        log_warning "代码拉取失败，使用当前代码继续部署"
    fi
}

# 构建和部署应用
deploy_application() {
    log_info "部署应用..."
    
    cd "$PROJECT_DIR"
    
    # 停止现有服务
    log_info "停止现有服务..."
    $DOCKER_COMPOSE_CMD -f "$DOCKER_COMPOSE_FILE" down
    
    # 清理未使用的镜像和容器
    log_info "清理 Docker 资源..."
    docker system prune -f
    
    # 构建新镜像
    log_info "构建应用镜像..."
    $DOCKER_COMPOSE_CMD -f "$DOCKER_COMPOSE_FILE" build --no-cache
    
    # 启动服务
    log_info "启动服务..."
    $DOCKER_COMPOSE_CMD -f "$DOCKER_COMPOSE_FILE" up -d
    
    # 等待服务启动
    log_info "等待服务启动..."
    sleep 30
    
    log_success "应用部署完成"
}

# 健康检查
health_check() {
    log_info "执行健康检查..."
    
    local max_attempts=10
    local attempt=1
    local health_url="http://localhost:8080/health"
    
    while [[ $attempt -le $max_attempts ]]; do
        log_info "健康检查尝试 $attempt/$max_attempts..."
        
        if curl -f "$health_url" &> /dev/null; then
            log_success "健康检查通过"
            return 0
        fi
        
        sleep 10
        ((attempt++))
    done
    
    log_error "健康检查失败"
    return 1
}

# 显示部署状态
show_status() {
    log_info "显示部署状态..."
    
    cd "$PROJECT_DIR"
    
    echo ""
    echo "=== 服务状态 ==="
    $DOCKER_COMPOSE_CMD -f "$DOCKER_COMPOSE_FILE" ps
    
    echo ""
    echo "=== 服务日志 (最近 20 行) ==="
    $DOCKER_COMPOSE_CMD -f "$DOCKER_COMPOSE_FILE" logs --tail=20
    
    echo ""
    echo "=== 系统资源使用情况 ==="
    echo "内存使用:"
    free -h
    echo ""
    echo "磁盘使用:"
    df -h
    echo ""
    echo "Docker 资源使用:"
    docker system df
}

# 主函数
main() {
    echo "=== RustIM 远程部署脚本 ==="
    echo "环境: $ENVIRONMENT"
    echo "项目目录: $PROJECT_DIR"
    echo "Docker Compose 文件: $DOCKER_COMPOSE_FILE"
    echo ""
    
    # 执行部署步骤
    check_requirements
    detect_docker_compose
    create_backup
    setup_environment
    pull_latest_code
    deploy_application
    
    # 健康检查
    if health_check; then
        log_success "部署成功完成！"
        show_status
    else
        log_error "部署失败，请检查日志"
        show_status
        exit 1
    fi
}

# 执行主函数
main "$@" 