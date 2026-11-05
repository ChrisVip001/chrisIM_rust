#!/bin/bash
set -e

# 定义支持的服务列表
# 必须与 Dockerfile 实际打包进 /app/bin 的二进制保持一致，
# 否则容器会以 "服务不存在" 直接退出。
# 注意：oss 是库 crate（被 api-gateway 依赖），没有独立进程。
SUPPORTED_SERVICES=(
    "api-gateway"
    "relation-service"
    "msg-server"
    "msg-gateway"
    "all"
)

# 显示帮助信息
show_help() {
    echo "RustIM Docker容器入口脚本"
    echo "用法: $0 [服务名]"
    echo ""
    echo "可用服务:"
    for service in "${SUPPORTED_SERVICES[@]}"; do
        echo "  - $service"
    done
    echo ""
    echo "示例:"
    echo "  $0 api-gateway      # 启动API网关服务"
    echo "  $0 all              # 启动所有服务"
}

# 启动单个服务
start_service() {
    local service=$1
    echo "启动服务: $service"
    if [[ -x "/app/bin/$service" ]]; then
        exec "/app/bin/$service"
    else
        echo "错误: 服务 $service 不存在或不可执行"
        exit 1
    fi
}

# 启动所有服务（仅用于开发环境）
start_all_services() {
    echo "启动所有服务（开发模式）"
    
    # 使用ampersand启动所有后台服务
    for service in "${SUPPORTED_SERVICES[@]}"; do
        if [[ "$service" != "all" && -x "/app/bin/$service" ]]; then
            echo "启动服务: $service"
            "/app/bin/$service" &
        fi
    done
    
    # 等待所有子进程
    wait
}

# 主函数
main() {
    # 检查是否请求帮助
    if [[ "$1" == "--help" || "$1" == "-h" ]]; then
        show_help
        exit 0
    fi

    # 服务注册到 Consul 的地址必须是容器 IP：配置文件里的 127.0.0.1
    # 会让服务发现把流量导向调用方自己，多副本时还会出现 ID 冲突。
    # k8s 由 ADVERTISE_HOST=status.podIP 注入；compose 在此按容器 IP 兜底。
    if [[ -z "${ADVERTISE_HOST:-}" ]]; then
        local ip
        ip=$(hostname -i 2>/dev/null | awk '{print $1}' || true)
        if [[ -n "$ip" && "$ip" != "127.0.0.1" ]]; then
            export ADVERTISE_HOST="$ip"
            echo "服务注册地址: $ADVERTISE_HOST"
        fi
    fi
    
    # 检查提供的服务是否受支持
    local service=${1:-"api-gateway"}  # 默认启动网关服务
    local is_supported=false
    
    for supported_service in "${SUPPORTED_SERVICES[@]}"; do
        if [[ "$service" == "$supported_service" ]]; then
            is_supported=true
            break
        fi
    done
    
    if [[ "$is_supported" == "false" ]]; then
        echo "错误: 不支持的服务 '$service'"
        show_help
        exit 1
    fi
    
    # 启动请求的服务
    if [[ "$service" == "all" ]]; then
        start_all_services
    else
        start_service "$service"
    fi
}

# 运行主函数
main "$@" 