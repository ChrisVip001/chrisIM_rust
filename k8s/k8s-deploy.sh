#!/bin/bash

# RustIM Kubernetes 部署脚本

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

# 显示帮助信息
show_help() {
    cat << EOF
RustIM Kubernetes 部署脚本

用法: $0 [选项] [操作]

操作:
  deploy      部署所有服务 (默认)
  undeploy    删除所有服务
  status      查看部署状态
  logs        查看服务日志
  scale       扩展服务实例

选项:
  -h, --help              显示帮助信息
  -n, --namespace NAME    指定命名空间 (默认: rustim)
  -i, --image TAG         指定镜像标签 (默认: latest)
  -f, --force             强制重新部署
  -w, --wait              等待部署完成
  --dry-run               仅显示将要执行的操作
  --service SERVICE       指定特定服务
  --replicas N            指定副本数量

示例:
  $0 deploy               # 部署所有服务
  $0 undeploy             # 删除所有服务
  $0 status               # 查看状态
  $0 scale --service api-gateway --replicas 5  # 扩展API网关到5个实例
  $0 logs --service api-gateway                 # 查看API网关日志

EOF
}

# 检查依赖
check_dependencies() {
    log_info "检查依赖..."
    
    # 检查 kubectl
    if ! command -v kubectl &> /dev/null; then
        log_error "kubectl 未安装，请先安装 kubectl"
        exit 1
    fi
    
    # 检查集群连接
    if ! kubectl cluster-info &> /dev/null; then
        log_error "无法连接到 Kubernetes 集群"
        exit 1
    fi
    
    log_success "依赖检查通过"
}

# 创建命名空间
create_namespace() {
    local namespace=$1
    
    log_info "创建命名空间: $namespace"
    
    if kubectl get namespace "$namespace" &> /dev/null; then
        log_warning "命名空间 $namespace 已存在"
    else
        kubectl apply -f k8s/namespace.yaml
        log_success "命名空间 $namespace 已创建"
    fi
}

# 部署基础设施
deploy_infrastructure() {
    local namespace=$1
    local dry_run=$2
    
    log_info "部署基础设施..."
    
    local kubectl_args=""
    if [[ "$dry_run" == "true" ]]; then
        kubectl_args="--dry-run=client"
    fi
    
    # 部署 ConfigMap 和 Secrets
    kubectl apply -f k8s/configmap.yaml $kubectl_args
    kubectl apply -f k8s/secrets.yaml $kubectl_args
    
    # 部署数据库
    if [[ -f k8s/postgres.yaml ]]; then
        kubectl apply -f k8s/postgres.yaml $kubectl_args
    fi
    
    # 部署 Redis
    if [[ -f k8s/redis.yaml ]]; then
        kubectl apply -f k8s/redis.yaml $kubectl_args
    fi
    
    # 部署 Kafka
    if [[ -f k8s/kafka.yaml ]]; then
        kubectl apply -f k8s/kafka.yaml $kubectl_args
    fi
    
    log_success "基础设施部署完成"
}

# 部署应用服务
deploy_services() {
    local namespace=$1
    local image_tag=$2
    local dry_run=$3
    local specific_service=$4
    
    log_info "部署应用服务..."
    
    local kubectl_args=""
    if [[ "$dry_run" == "true" ]]; then
        kubectl_args="--dry-run=client"
    fi
    
    # 更新镜像标签
    if [[ "$image_tag" != "latest" ]]; then
        log_info "更新镜像标签为: $image_tag"
        # 这里可以添加镜像标签替换逻辑
    fi
    
    # 部署服务
    local services=("api-gateway" "msg-gateway" "user-service" "friend-service" "group-service" "msg-server" "oss")
    
    for service in "${services[@]}"; do
        if [[ -n "$specific_service" && "$service" != "$specific_service" ]]; then
            continue
        fi
        
        if [[ -f "k8s/$service.yaml" ]]; then
            log_info "部署服务: $service"
            kubectl apply -f "k8s/$service.yaml" $kubectl_args
        else
            log_warning "服务配置文件不存在: k8s/$service.yaml"
        fi
    done
    
    # 部署 Nginx 负载均衡器
    if [[ -f k8s/nginx.yaml ]]; then
        kubectl apply -f k8s/nginx.yaml $kubectl_args
    fi
    
    log_success "应用服务部署完成"
}

# 等待部署完成
wait_for_deployment() {
    local namespace=$1
    local timeout=${2:-300}
    
    log_info "等待部署完成 (超时: ${timeout}s)..."
    
    # 等待所有 Deployment 就绪
    if kubectl wait --for=condition=available --timeout=${timeout}s deployment --all -n "$namespace"; then
        log_success "所有服务部署完成"
    else
        log_error "部署超时或失败"
        return 1
    fi
}

# 删除部署
undeploy() {
    local namespace=$1
    local dry_run=$2
    
    log_info "删除部署..."
    
    local kubectl_args=""
    if [[ "$dry_run" == "true" ]]; then
        kubectl_args="--dry-run=client"
    fi
    
    # 删除所有资源
    if [[ -d k8s ]]; then
        kubectl delete -f k8s/ $kubectl_args --ignore-not-found=true
    fi
    
    # 删除命名空间
    kubectl delete namespace "$namespace" $kubectl_args --ignore-not-found=true
    
    log_success "删除完成"
}

# 显示状态
show_status() {
    local namespace=$1
    local specific_service=$2
    
    log_info "服务状态:"
    
    if [[ -n "$specific_service" ]]; then
        kubectl get pods -n "$namespace" -l app="$specific_service"
        kubectl get svc -n "$namespace" -l app="$specific_service"
        kubectl get deployment -n "$namespace" -l app="$specific_service"
    else
        kubectl get all -n "$namespace"
    fi
    
    echo ""
    log_info "Pod 详细状态:"
    kubectl get pods -n "$namespace" -o wide
    
    echo ""
    log_info "服务端点:"
    kubectl get svc -n "$namespace"
}

# 显示日志
show_logs() {
    local namespace=$1
    local specific_service=$2
    local follow=${3:-false}
    
    if [[ -n "$specific_service" ]]; then
        local follow_arg=""
        if [[ "$follow" == "true" ]]; then
            follow_arg="-f"
        fi
        
        log_info "显示服务日志: $specific_service"
        kubectl logs -n "$namespace" -l app="$specific_service" $follow_arg --tail=100
    else
        log_info "显示所有服务日志:"
        kubectl logs -n "$namespace" --all-containers=true --tail=50
    fi
}

# 扩展服务
scale_service() {
    local namespace=$1
    local service=$2
    local replicas=$3
    local dry_run=$4
    
    log_info "扩展服务 $service 到 $replicas 个实例"
    
    local kubectl_args=""
    if [[ "$dry_run" == "true" ]]; then
        kubectl_args="--dry-run=client"
    fi
    
    kubectl scale deployment "$service" --replicas="$replicas" -n "$namespace" $kubectl_args
    
    if [[ "$dry_run" != "true" ]]; then
        kubectl rollout status deployment "$service" -n "$namespace"
        log_success "服务 $service 扩展完成"
    fi
}

# 主函数
main() {
    local operation="deploy"
    local namespace="rustim"
    local image_tag="latest"
    local force=false
    local wait=false
    local dry_run=false
    local specific_service=""
    local replicas=""
    local follow_logs=false
    
    # 解析命令行参数
    while [[ $# -gt 0 ]]; do
        case $1 in
            -h|--help)
                show_help
                exit 0
                ;;
            -n|--namespace)
                namespace="$2"
                shift 2
                ;;
            -i|--image)
                image_tag="$2"
                shift 2
                ;;
            -f|--force)
                force=true
                shift
                ;;
            -w|--wait)
                wait=true
                shift
                ;;
            --dry-run)
                dry_run=true
                shift
                ;;
            --service)
                specific_service="$2"
                shift 2
                ;;
            --replicas)
                replicas="$2"
                shift 2
                ;;
            --follow)
                follow_logs=true
                shift
                ;;
            deploy|undeploy|status|logs|scale)
                operation="$1"
                shift
                ;;
            *)
                log_error "未知参数: $1"
                show_help
                exit 1
                ;;
        esac
    done
    
    log_info "开始执行 Kubernetes 操作: $operation"
    
    # 检查依赖
    check_dependencies
    
    case $operation in
        deploy)
            create_namespace "$namespace"
            deploy_infrastructure "$namespace" "$dry_run"
            deploy_services "$namespace" "$image_tag" "$dry_run" "$specific_service"
            
            if [[ "$wait" == "true" && "$dry_run" != "true" ]]; then
                wait_for_deployment "$namespace"
            fi
            
            if [[ "$dry_run" != "true" ]]; then
                show_status "$namespace"
            fi
            ;;
        undeploy)
            undeploy "$namespace" "$dry_run"
            ;;
        status)
            show_status "$namespace" "$specific_service"
            ;;
        logs)
            show_logs "$namespace" "$specific_service" "$follow_logs"
            ;;
        scale)
            if [[ -z "$specific_service" || -z "$replicas" ]]; then
                log_error "扩展操作需要指定 --service 和 --replicas 参数"
                exit 1
            fi
            scale_service "$namespace" "$specific_service" "$replicas" "$dry_run"
            ;;
        *)
            log_error "未知操作: $operation"
            show_help
            exit 1
            ;;
    esac
    
    log_success "操作完成: $operation"
}

# 执行主函数
main "$@" 