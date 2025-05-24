#!/bin/bash

# RustIM 远程部署脚本 - 针对腾讯云 OpenCloudOS 优化
# 支持多环境部署：staging 和 production
# 集成快速构建优化功能

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
DOCKER_COMPOSE_FILE="docker-compose.yml"
BACKUP_DIR="/home/$(whoami)/backups"
GIT_REMOTE_URL="https://github.com/ChrisVip001/chrisIM_rust"

# 构建优化配置
USE_FAST_BUILD=true
USE_CHINA_MIRROR=false
CLEAN_BUILD=false
BUILD_PARALLEL_JOBS=$(nproc)
DOCKER_BUILDKIT=1
COMPOSE_DOCKER_CLI_BUILD=1

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
    echo "部署选项:"
    echo "  -e, --environment ENV    部署环境 (staging|production) [默认: production]"
    echo "  -d, --directory DIR      项目目录 [默认: $PROJECT_DIR]"
    echo "  -f, --compose-file FILE  Docker Compose 文件 [默认: $DOCKER_COMPOSE_FILE]"
    echo "  -b, --backup-dir DIR     备份目录 [默认: $BACKUP_DIR]"
    echo ""
    echo "构建优化选项:"
    echo "  --fast-build            启用快速构建优化 [默认: 启用]"
    echo "  --no-fast-build         禁用快速构建优化"
    echo "  --use-china-mirror      使用中国镜像源加速"
    echo "  --clean-build           清理所有缓存后构建"
    echo "  --parallel JOBS         并行构建任务数 [默认: $BUILD_PARALLEL_JOBS]"
    echo "  --no-cache              不使用构建缓存"
    echo ""
    echo "其他选项:"
    echo "  -h, --help              显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  $0 -e staging                    # 部署到staging环境"
    echo "  $0 -e production --fast-build    # 生产环境快速构建部署"
    echo "  $0 --use-china-mirror            # 使用中国镜像源加速"
    echo "  $0 --clean-build --parallel 4    # 清理缓存并使用4个并行任务"
}

# 解析命令行参数
USE_BUILD_CACHE=true

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
        --fast-build)
            USE_FAST_BUILD=true
            shift
            ;;
        --no-fast-build)
            USE_FAST_BUILD=false
            shift
            ;;
        --use-china-mirror)
            USE_CHINA_MIRROR=true
            shift
            ;;
        --clean-build)
            CLEAN_BUILD=true
            shift
            ;;
        --parallel)
            BUILD_PARALLEL_JOBS="$2"
            shift 2
            ;;
        --no-cache)
            USE_BUILD_CACHE=false
            shift
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

# 显示部署配置信息
show_deploy_config() {
    log_info "部署配置信息:"
    echo "  - 环境: $ENVIRONMENT"
    echo "  - 项目目录: $PROJECT_DIR"
    echo "  - Docker Compose 文件: $DOCKER_COMPOSE_FILE"
    echo "  - 备份目录: $BACKUP_DIR"
    echo ""
    log_info "构建优化配置:"
    echo "  - 快速构建: $USE_FAST_BUILD"
    echo "  - 中国镜像源: $USE_CHINA_MIRROR"
    echo "  - 清理构建: $CLEAN_BUILD"
    echo "  - 并行任务数: $BUILD_PARALLEL_JOBS"
    echo "  - 使用缓存: $USE_BUILD_CACHE"
    echo "  - BuildKit: $DOCKER_BUILDKIT"
    echo ""
}

# 检查系统要求
check_requirements() {
    log_info "检查系统要求..."

    # 检查 Docker
    if ! command -v docker &> /dev/null; then
        log_error "Docker 未安装"
        exit 1
    fi

    if ! docker info &> /dev/null; then
        log_error "Docker 服务未运行"
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

# 优化 Docker 构建环境
optimize_docker_build() {
    if [[ "$USE_FAST_BUILD" != "true" ]]; then
        log_info "跳过 Docker 构建优化"
        return 0
    fi

    log_info "优化 Docker 构建环境..."

    # 启用 BuildKit
    export DOCKER_BUILDKIT=1
    export COMPOSE_DOCKER_CLI_BUILD=1

    # 设置并行构建
    export DOCKER_BUILD_PARALLEL=$BUILD_PARALLEL_JOBS

    # 预热构建缓存
    log_info "预热构建缓存..."
    docker pull rust:1.75-slim-bullseye &
    docker pull debian:bullseye-slim &
    wait

    log_success "Docker 构建环境优化完成"
}

# 清理构建缓存和镜像
clean_build_cache() {
    if [[ "$CLEAN_BUILD" != "true" ]]; then
        return 0
    fi

    log_info "清理构建缓存和镜像..."

    # 停止所有容器
    log_info "停止所有相关容器..."
    $DOCKER_COMPOSE_CMD -f "$PROJECT_DIR/$DOCKER_COMPOSE_FILE" down --remove-orphans 2>/dev/null || true

    # 清理构建缓存
    log_info "清理 Docker 构建缓存..."
    docker builder prune -f

    # 清理未使用的镜像
    log_info "清理未使用的镜像..."
    docker image prune -f

    # 清理 RustIM 相关镜像
    log_info "清理 RustIM 相关镜像..."
    docker images | grep -E "(rustim|rust-im)" | awk '{print $3}' | xargs -r docker rmi -f 2>/dev/null || true

    # 清理系统资源
    log_info "清理系统资源..."
    docker system prune -f

    log_success "清理完成"
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

    # 配置 Git 网络设置
    log_info "配置 Git 网络设置..."
    git config --global http.lowSpeedLimit 1000
    git config --global http.lowSpeedTime 300
    git config --global http.postBuffer 524288000
    git config --global core.compression 0

    # 检查是否是 Git 仓库
    if [[ ! -d ".git" ]]; then
        log_warning "当前目录不是 Git 仓库，正在初始化..."

        # 初始化 Git 仓库
        git init

        # 添加远程仓库（需要用户提供）
        if [[ -z "${GIT_REMOTE_URL:-}" ]]; then
            log_error "请设置环境变量 GIT_REMOTE_URL 或手动配置 Git 仓库"
            log_info "示例："
            log_info "  export GIT_REMOTE_URL=https://github.com/username/rust-im.git"
            log_info "  或者手动执行："
            log_info "  cd $PROJECT_DIR"
            log_info "  git init"
            log_info "  git remote add origin <your-repo-url>"
            log_info "  git fetch origin"
            log_info "  git checkout <branch-name>"
            exit 1
        fi

        # 添加远程仓库
        git remote add origin "$GIT_REMOTE_URL"

        # 获取远程分支
        log_info "获取远程分支信息..."
        git fetch origin

        # 根据环境确定分支
        local branch
        if [[ "$ENVIRONMENT" == "staging" ]]; then
            branch="develop"
        else
            branch="release"
        fi

        # 检查远程分支是否存在
        if git ls-remote --heads origin "$branch" | grep -q "$branch"; then
            log_info "检出 $branch 分支..."
            git checkout -b "$branch" "origin/$branch"
        else
            log_error "远程分支 $branch 不存在"
            log_info "可用的远程分支："
            git ls-remote --heads origin
            exit 1
        fi

        log_success "Git 仓库初始化完成"
        return 0
    fi

    # 根据环境确定分支
    local branch
    if [[ "$ENVIRONMENT" == "staging" ]]; then
        branch="develop"
    else
        branch="release"
    fi

    # 检查是否有未提交的更改
    if ! git diff --quiet || ! git diff --cached --quiet; then
        log_warning "检测到本地文件有更改，正在处理..."

        # 备份重要的配置文件
        local backup_timestamp=$(date +"%Y%m%d_%H%M%S")
        local temp_backup_dir="/tmp/rustim_config_backup_${backup_timestamp}"
        mkdir -p "$temp_backup_dir"

        # 备份 .env 文件（如果存在且有更改）
        if [[ -f ".env" ]] && ! git diff --quiet .env 2>/dev/null; then
            log_info "备份本地 .env 文件..."
            cp ".env" "$temp_backup_dir/.env.local"
        fi

        # 备份其他重要配置文件
        for config_file in "docker-compose.override.yml" "config/nginx.conf" "config/redis.conf"; do
            if [[ -f "$config_file" ]] && ! git diff --quiet "$config_file" 2>/dev/null; then
                log_info "备份 $config_file..."
                mkdir -p "$temp_backup_dir/$(dirname "$config_file")"
                cp "$config_file" "$temp_backup_dir/$config_file"
            fi
        done

        # 暂存本地更改
        log_info "暂存本地更改..."
        git stash push -m "Auto-stash before deployment at $(date)"

        log_success "本地更改已暂存，备份保存在: $temp_backup_dir"
    fi

    # 检查当前分支
    local current_branch=$(git branch --show-current)
    if [[ "$current_branch" != "$branch" ]]; then
        log_info "切换到 $branch 分支..."

        # 检查分支是否存在
        if git show-ref --verify --quiet "refs/heads/$branch"; then
            git checkout "$branch"
        elif git show-ref --verify --quiet "refs/remotes/origin/$branch"; then
            git checkout -b "$branch" "origin/$branch"
        else
            log_error "分支 $branch 不存在"
            log_info "可用的分支："
            git branch -a
            exit 1
        fi
    fi

    # 拉取最新代码 - 添加重试机制
    log_info "拉取 $branch 分支的最新代码..."

    local max_retries=3
    local retry_count=0
    local pull_success=false

    while [[ $retry_count -lt $max_retries ]]; do
        retry_count=$((retry_count + 1))
        log_info "尝试拉取代码 (第 $retry_count 次)..."

        # 尝试使用不同的方法拉取代码
        if [[ $retry_count -eq 1 ]]; then
            # 第一次尝试：正常拉取
            if timeout 300 git pull origin "$branch"; then
                pull_success=true
                break
            fi
        elif [[ $retry_count -eq 2 ]]; then
            # 第二次尝试：使用浅克隆
            log_warning "尝试使用浅克隆方式..."
            if timeout 300 git pull --depth=1 origin "$branch"; then
                pull_success=true
                break
            fi
        else
            # 第三次尝试：重置并强制拉取
            log_warning "尝试重置并强制拉取..."
            if timeout 300 git fetch origin "$branch" && git reset --hard "origin/$branch"; then
                pull_success=true
                break
            fi
        fi

        log_warning "第 $retry_count 次拉取失败，等待 10 秒后重试..."
        sleep 10
    done

    if [[ "$pull_success" != "true" ]]; then
        log_error "代码拉取失败，网络连接问题"
        log_info "尝试重新克隆仓库..."

        # 备份当前目录
        local backup_dir="${PROJECT_DIR}.backup.$(date +%Y%m%d_%H%M%S)"
        if [[ -d "$PROJECT_DIR" ]]; then
            log_info "备份当前项目目录到: $backup_dir"
            mv "$PROJECT_DIR" "$backup_dir"
        fi

        # 创建父目录
        mkdir -p "$(dirname "$PROJECT_DIR")"

        # 尝试重新克隆
        log_info "重新克隆项目..."
        if timeout 600 git clone "$GIT_REMOTE_URL" "$PROJECT_DIR"; then
            cd "$PROJECT_DIR"

            # 切换到目标分支
            if git show-ref --verify --quiet "refs/remotes/origin/$branch"; then
                git checkout -b "$branch" "origin/$branch" 2>/dev/null || git checkout "$branch"
                log_success "重新克隆成功，已切换到 $branch 分支"

                # 恢复重要的配置文件
                if [[ -d "$backup_dir" ]]; then
                    log_info "恢复配置文件..."
                    [[ -f "$backup_dir/.env" ]] && cp "$backup_dir/.env" ".env"
                    [[ -f "$backup_dir/docker-compose.override.yml" ]] && cp "$backup_dir/docker-compose.override.yml" "."
                    log_success "配置文件恢复完成"
                fi

                return 0
            else
                log_error "目标分支 $branch 不存在"
                # 恢复备份
                if [[ -d "$backup_dir" ]]; then
                    rm -rf "$PROJECT_DIR"
                    mv "$backup_dir" "$PROJECT_DIR"
                    log_info "已恢复原项目目录"
                fi
                exit 1
            fi
        else
            log_error "重新克隆也失败了"
            # 恢复备份
            if [[ -d "$backup_dir" ]]; then
                mv "$backup_dir" "$PROJECT_DIR"
                log_info "已恢复原项目目录"
            fi

            log_info "可能的解决方案："
            log_info "1. 检查网络连接: ping github.com"
            log_info "2. 配置代理: git config --global http.proxy http://proxy:port"
            log_info "3. 使用 SSH 克隆: git remote set-url origin git@github.com:username/repo.git"
            log_info "4. 手动下载代码并解压到项目目录"
            log_info "5. 跳过代码更新继续部署: export SKIP_GIT_PULL=true"

            # 检查是否设置了跳过 Git 拉取
            if [[ "${SKIP_GIT_PULL:-}" == "true" ]]; then
                log_warning "跳过 Git 拉取，使用当前代码继续部署"
                return 0
            fi

            exit 1
        fi
    fi

    # 如果有暂存的更改，询问是否恢复
    if git stash list | grep -q "Auto-stash before deployment"; then
        log_warning "检测到之前暂存的本地更改"
        log_info "暂存列表："
        git stash list | head -3

        # 在部署脚本中，我们通常不恢复暂存的更改
        # 因为环境配置应该通过 .env.staging 或 .env.production 来管理
        log_info "本地更改已暂存，如需恢复请手动执行："
        log_info "  git stash pop"
        log_info "注意：建议使用环境特定的配置文件（.env.staging, .env.production）"
    fi

    log_success "代码更新完成"
}
# 构建和部署应用
deploy_application() {
    log_info "部署应用..."

    cd "$PROJECT_DIR"

    # 停止现有服务
    log_info "停止现有服务..."
    $DOCKER_COMPOSE_CMD -f "$DOCKER_COMPOSE_FILE" down

    # 清理未使用的镜像和容器（如果不是清理构建）
    if [[ "$CLEAN_BUILD" != "true" ]]; then
        log_info "清理 Docker 资源..."
        docker system prune -f
    fi

    # 准备构建参数
    local build_args=""

    if [[ "$USE_CHINA_MIRROR" == "true" ]]; then
        build_args="$build_args --build-arg USE_CHINA_MIRROR=true"
        log_info "使用中国镜像源加速构建"
    fi

    if [[ "$USE_BUILD_CACHE" == "false" ]]; then
        build_args="$build_args --no-cache"
        log_warning "禁用构建缓存"
    fi

    # 构建新镜像
    log_info "构建应用镜像..."
    local build_cmd="$DOCKER_COMPOSE_CMD -f $DOCKER_COMPOSE_FILE build $build_args"
    local start_time=$(date +%s)

    if [[ "$USE_FAST_BUILD" == "true" ]]; then
        log_info "使用快速构建模式"
        log_info "执行构建命令: $build_cmd"

        # 后台执行构建并监控进度
        $build_cmd &
        local build_pid=$!

        # 监控构建进度
        monitor_build_progress $build_pid

        # 等待构建完成
        wait $build_pid
        local build_result=$?
    else
        log_info "使用标准构建模式"
        $build_cmd
        local build_result=$?
    fi

    local end_time=$(date +%s)
    local total_time=$((end_time - start_time))
    local minutes=$((total_time / 60))
    local seconds=$((total_time % 60))

    if [[ $build_result -eq 0 ]]; then
        log_success "镜像构建完成！用时: ${minutes}分${seconds}秒"
    else
        log_error "镜像构建失败！"
        exit 1
    fi

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

    if [[ "$USE_FAST_BUILD" == "true" ]]; then
        echo ""
        echo "=== 构建缓存信息 ==="
        docker builder du 2>/dev/null || echo "无法获取构建缓存信息"
    fi
}

# 显示优化建议
show_optimization_tips() {
    if [[ "$USE_FAST_BUILD" != "true" ]]; then
        return 0
    fi

    log_info "构建优化建议:"
    echo ""
    echo "🚀 进一步加速构建的方法:"
    echo "  1. 使用 SSD 硬盘存储 Docker 数据"
    echo "  2. 增加服务器内存和 CPU 核心数"
    echo "  3. 配置 Docker Hub 镜像加速器"
    echo "  4. 使用本地 Cargo 缓存目录挂载"
    echo "  5. 定期清理不必要的 Docker 镜像和容器"
    echo ""
    echo "🌐 网络优化:"
    echo "  1. 使用 --use-china-mirror 选项"
    echo "  2. 配置 HTTP/HTTPS 代理"
    echo "  3. 使用企业内部镜像仓库"
    echo ""
    echo "💾 缓存优化:"
    echo "  1. 保持 Cargo.lock 文件在版本控制中"
    echo "  2. 避免频繁使用 --clean-build"
    echo "  3. 合理使用 --parallel 参数"
}

# 主函数
main() {
    echo "=== RustIM 远程部署脚本 (集成快速构建) ==="
    echo ""

    # 显示配置信息
    show_deploy_config

    # 执行部署步骤
    check_requirements
    detect_docker_compose
    optimize_docker_build
    clean_build_cache
    create_backup
    setup_environment
    pull_latest_code
    deploy_application

    # 健康检查
    if health_check; then
        log_success "部署成功完成！"
        show_status
        show_optimization_tips
    else
        log_error "部署失败，请检查日志"
        show_status
        exit 1
    fi
}

# 执行主函数
main "$@" 