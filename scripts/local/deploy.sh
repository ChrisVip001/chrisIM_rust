#!/bin/bash

# RustIM 部署脚本
# 支持开发环境和生产环境部署

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

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

# 显示帮助信息
show_help() {
    cat << EOF
RustIM 部署脚本

用法: $0 [选项] [环境]

环境:
  dev         开发环境部署 (默认)
  prod        生产环境部署
  test        测试环境部署

选项:
  -h, --help              显示帮助信息
  -c, --clean             清理现有容器和镜像
  -b, --build             强制重新构建镜像
  -d, --detach            后台运行 (默认)
  -f, --foreground        前台运行
  -s, --scale SERVICE=N   扩展指定服务实例数
  --no-deps               不启动依赖服务
  --pull                  拉取最新镜像
  --logs                  显示日志
  --status                显示服务状态

示例:
  $0 dev                  # 开发环境部署
  $0 prod -b              # 生产环境部署并重新构建
  $0 dev --clean          # 清理后部署开发环境
  $0 prod -s api-gateway=3 # 生产环境部署并扩展API网关到3个实例

EOF
}

# 检查依赖
check_dependencies() {
    log_info "检查依赖..."
    
    # 检查 Docker
    if ! command -v docker &> /dev/null; then
        log_error "Docker 未安装，请先安装 Docker"
        exit 1
    fi
    
    # 检查 Docker 是否运行
    if ! docker info &> /dev/null; then
        log_error "Docker 服务未运行，请启动 Docker"
        exit 1
    fi
    
    log_success "依赖检查通过"
}

# 设置环境变量
setup_environment() {
    local env=$1
    log_info "设置 $env 环境变量..."
    
    # 创建 .env 文件如果不存在
    if [[ ! -f .env ]]; then
        log_warning ".env 文件不存在，创建默认配置..."
        cat > .env << EOF
# 数据库配置
POSTGRES_PASSWORD=rustim_secure_password_$(date +%s)
DATABASE_URL=postgresql://rustim:\${POSTGRES_PASSWORD}@localhost:5432/rustim

# Redis 配置
REDIS_URL=redis://localhost:6379

# Kafka 配置
KAFKA_BROKERS=localhost:9092

# JWT 配置
JWT_SECRET=your_super_secure_jwt_secret_key_$(openssl rand -hex 32)

# AWS S3 配置 (可选)
AWS_ACCESS_KEY_ID=your_aws_access_key
AWS_SECRET_ACCESS_KEY=your_aws_secret_key
AWS_REGION=us-east-1

# 日志级别
RUST_LOG=info
EOF
        log_success "已创建默认 .env 文件，请根据需要修改配置"
    fi
    
    # 加载环境变量
    if [[ -f .env ]]; then
        export $(cat .env | grep -v '^#' | xargs)
    fi
}

# 清理环境
clean_environment() {
    log_info "清理现有环境..."
    
    # 停止并删除容器
    $DOCKER_COMPOSE_CMD down --remove-orphans 2>/dev/null || true
    $DOCKER_COMPOSE_CMD -f docker-compose.prod.yml down --remove-orphans 2>/dev/null || true
    
    # 删除未使用的镜像
    docker image prune -f
    
    # 删除未使用的网络
    docker network prune -f
    
    log_success "环境清理完成"
}

# 构建镜像
build_images() {
    local env=$1
    local force_build=$2
    
    if [[ "$force_build" == "true" ]]; then
        log_info "强制重新构建镜像..."
        if [[ "$env" == "prod" ]]; then
            $DOCKER_COMPOSE_CMD -f docker-compose.yml -f docker-compose.prod.yml build --no-cache
        else
            $DOCKER_COMPOSE_CMD build --no-cache
        fi
    else
        log_info "构建镜像..."
        if [[ "$env" == "prod" ]]; then
            $DOCKER_COMPOSE_CMD -f docker-compose.yml -f docker-compose.prod.yml build
        else
            $DOCKER_COMPOSE_CMD build
        fi
    fi
    
    log_success "镜像构建完成"
}

# 创建必要的目录
create_directories() {
    log_info "创建必要的目录..."
    
    mkdir -p logs/nginx
    mkdir -p uploads
    mkdir -p config/ssl
    
    # 设置权限
    chmod 755 logs uploads
    
    log_success "目录创建完成"
}

# 初始化数据库
init_database() {
    log_info "初始化数据库..."
    
    # 创建数据库初始化脚本
    cat > scripts/init-db.sql << 'EOF'
-- 创建扩展
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pg_stat_statements";

-- 创建用户表
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    username VARCHAR(50) UNIQUE NOT NULL,
    email VARCHAR(100) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    avatar_url VARCHAR(255),
    status INTEGER DEFAULT 1,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- 创建好友关系表
CREATE TABLE IF NOT EXISTS friendships (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    friend_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status INTEGER DEFAULT 0, -- 0: pending, 1: accepted, 2: blocked
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(user_id, friend_id)
);

-- 创建群组表
CREATE TABLE IF NOT EXISTS groups (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name VARCHAR(100) NOT NULL,
    description TEXT,
    avatar_url VARCHAR(255),
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    max_members INTEGER DEFAULT 500,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

-- 创建群组成员表
CREATE TABLE IF NOT EXISTS group_members (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    group_id UUID NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role INTEGER DEFAULT 0, -- 0: member, 1: admin, 2: owner
    joined_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(group_id, user_id)
);

-- 创建消息表
CREATE TABLE IF NOT EXISTS messages (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    sender_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    receiver_id UUID REFERENCES users(id) ON DELETE CASCADE,
    group_id UUID REFERENCES groups(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    message_type INTEGER DEFAULT 0, -- 0: text, 1: image, 2: file, 3: voice
    file_url VARCHAR(255),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    CHECK ((receiver_id IS NOT NULL AND group_id IS NULL) OR (receiver_id IS NULL AND group_id IS NOT NULL))
);

-- 创建索引
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_friendships_user_id ON friendships(user_id);
CREATE INDEX IF NOT EXISTS idx_friendships_friend_id ON friendships(friend_id);
CREATE INDEX IF NOT EXISTS idx_groups_owner_id ON groups(owner_id);
CREATE INDEX IF NOT EXISTS idx_group_members_group_id ON group_members(group_id);
CREATE INDEX IF NOT EXISTS idx_group_members_user_id ON group_members(user_id);
CREATE INDEX IF NOT EXISTS idx_messages_sender_id ON messages(sender_id);
CREATE INDEX IF NOT EXISTS idx_messages_receiver_id ON messages(receiver_id);
CREATE INDEX IF NOT EXISTS idx_messages_group_id ON messages(group_id);
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at);
EOF
    
    log_success "数据库初始化脚本已创建"
}

# 创建 Redis 配置
create_redis_config() {
    log_info "创建 Redis 配置..."
    
    cat > config/redis.conf << 'EOF'
# Redis 生产环境配置
bind 0.0.0.0
port 6379
timeout 300
keepalive 60
maxmemory 512mb
maxmemory-policy allkeys-lru
save 900 1
save 300 10
save 60 10000
rdbcompression yes
rdbchecksum yes
dbfilename dump.rdb
dir /data
appendonly yes
appendfsync everysec
auto-aof-rewrite-percentage 100
auto-aof-rewrite-min-size 64mb
EOF
    
    log_success "Redis 配置已创建"
}

# 创建 Nginx 配置
create_nginx_config() {
    log_info "创建 Nginx 配置..."
    
    cat > config/nginx.conf << 'EOF'
events {
    worker_connections 1024;
}

http {
    upstream api_backend {
        server api-gateway:8080;
    }
    
    upstream ws_backend {
        server msg-gateway:8085;
    }
    
    server {
        listen 80;
        server_name localhost;
        
        # API 请求代理
        location /api/ {
            proxy_pass http://api_backend;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $scheme;
        }
        
        # WebSocket 代理
        location /ws/ {
            proxy_pass http://ws_backend;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $scheme;
        }
        
        # 健康检查
        location /health {
            proxy_pass http://api_backend/health;
        }
    }
}
EOF
    
    log_success "Nginx 配置已创建"
}

# 部署服务
deploy_services() {
    local env=$1
    local detach=$2
    local scale_services=$3
    local no_deps=$4
    local pull_images=$5
    
    log_info "部署 $env 环境服务..."
    
    local compose_args=""
    local compose_files="-f docker-compose.yml"
    
    if [[ "$env" == "prod" ]]; then
        compose_files="$compose_files -f docker-compose.prod.yml"
    elif [[ "$env" == "test" ]]; then
        compose_files="$compose_files -f docker-compose.test.yml"
    fi
    
    if [[ "$detach" == "true" ]]; then
        compose_args="$compose_args -d"
    fi
    
    if [[ "$no_deps" == "true" ]]; then
        compose_args="$compose_args --no-deps"
    fi
    
    if [[ "$pull_images" == "true" ]]; then
        log_info "拉取最新镜像..."
        $DOCKER_COMPOSE_CMD $compose_files pull
    fi
    
    # 启动服务
    $DOCKER_COMPOSE_CMD $compose_files up $compose_args
    
    # 扩展服务
    if [[ -n "$scale_services" ]]; then
        log_info "扩展服务: $scale_services"
        $DOCKER_COMPOSE_CMD $compose_files up -d --scale $scale_services
    fi
    
    log_success "$env 环境部署完成"
}

# 显示服务状态
show_status() {
    log_info "服务状态:"
    $DOCKER_COMPOSE_CMD ps
    
    echo ""
    log_info "服务健康状态:"
    
    # 检查各服务健康状态
    services=("api-gateway:8080" "msg-gateway:8085" "user-service:50001" "friend-service:50002" "group-service:50003" "msg-server:50004" "oss:50005")
    
    for service in "${services[@]}"; do
        name=$(echo $service | cut -d: -f1)
        port=$(echo $service | cut -d: -f2)
        
        if curl -f -s "http://localhost:$port/health" > /dev/null 2>&1; then
            log_success "$name: 健康"
        else
            log_error "$name: 不健康"
        fi
    done
}

# 显示日志
show_logs() {
    local service=$1
    
    if [[ -n "$service" ]]; then
        $DOCKER_COMPOSE_CMD logs -f "$service"
    else
        $DOCKER_COMPOSE_CMD logs -f
    fi
}

# 主函数
main() {
    local env="dev"
    local clean=false
    local build=false
    local detach=true
    local scale_services=""
    local no_deps=false
    local pull_images=false
    local show_logs_flag=false
    local show_status_flag=false
    
    # 解析命令行参数
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                show_help
                exit 0
                ;;
            -c|--clean)
                clean=true
                shift
                ;;
            -b|--build)
                build=true
                shift
                ;;
            -d|--detach)
                detach=true
                shift
                ;;
            -f|--foreground)
                detach=false
                shift
                ;;
            -s|--scale)
                scale_services="$2"
                shift 2
                ;;
            --no-deps)
                no_deps=true
                shift
                ;;
            --pull)
                pull_images=true
                shift
                ;;
            --logs)
                show_logs_flag=true
                shift
                ;;
            --status)
                show_status_flag=true
                shift
                ;;
            dev|prod|test)
                env="$1"
                shift
                ;;
            *)
                log_error "未知参数: $1"
                show_help
                exit 1
                ;;
        esac
    done
    
    # 如果只是查看状态或日志，直接执行
    if [[ "$show_status_flag" == "true" ]]; then
        show_status
        exit 0
    fi
    
    if [[ "$show_logs_flag" == "true" ]]; then
        show_logs
        exit 0
    fi
    
    log_info "开始部署 RustIM $env 环境..."
    
    # 检查依赖
    check_dependencies
    
    # 检测 Docker Compose 版本
    detect_docker_compose
    
    # 设置环境变量
    setup_environment "$env"
    
    # 清理环境（如果需要）
    if [[ "$clean" == "true" ]]; then
        clean_environment
    fi
    
    # 创建必要的目录和配置
    create_directories
    init_database
    create_redis_config
    create_nginx_config
    
    # 构建镜像
    build_images "$env" "$build"
    
    # 部署服务
    deploy_services "$env" "$detach" "$scale_services" "$no_deps" "$pull_images"
    
    # 等待服务启动
    if [[ "$detach" == "true" ]]; then
        log_info "等待服务启动..."
        sleep 30
        
        # 显示服务状态
        show_status
        
        echo ""
        log_success "RustIM $env 环境部署完成！"
        log_info "访问地址:"
        log_info "  API Gateway: http://localhost:8080"
        log_info "  WebSocket Gateway: ws://localhost:8085"
        log_info "  Nginx (如果启用): http://localhost"
        echo ""
        log_info "查看日志: $0 --logs"
        log_info "查看状态: $0 --status"
        log_info "停止服务: $DOCKER_COMPOSE_CMD down"
    fi
}

# 执行主函数
main "$@" 