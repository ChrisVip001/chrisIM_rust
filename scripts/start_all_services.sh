#!/bin/bash

# 确保在项目根目录执行
cd "$(dirname "$0")/.." || exit

# 创建日志目录
mkdir -p logs

# 定义要启动的服务列表
SERVICES=(
  "user-service"
  "group-service"
  "friend-service"
  "msg-server"
  "msg-gateway"
  "api-gateway"
  "msg-storage"
  "oss"
)

# 定义颜色输出
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# 检查服务是否正在运行
check_service() {
  local service=$1
  if pgrep -f "target/release/$service" > /dev/null; then
    return 0 # 已在运行
  else
    return 1 # 未在运行
  fi
}

# 启动所有服务
start_all() {
  echo -e "${GREEN}开始启动所有服务...${NC}"
  
  # 先编译所有服务
  echo -e "${GREEN}正在编译所有服务 (release 模式)...${NC}"
  cargo build --release
  
  if [ $? -ne 0 ]; then
    echo -e "${RED}编译失败，请检查错误信息${NC}"
    exit 1
  fi
  
  # 启动每个服务
  for service in "${SERVICES[@]}"; do
    if check_service "$service"; then
      echo -e "${GREEN}服务 $service 已经在运行中${NC}"
    else
      echo -e "${GREEN}正在启动 $service...${NC}"
      nohup "target/release/$service" > "logs/$service.log" 2>&1 &
      
      # 保存PID到文件
      echo $! > "logs/$service.pid"
      echo -e "${GREEN}服务 $service 已在后台启动，PID: $!${NC}"
    fi
  done
  
  echo -e "${GREEN}所有服务已启动完成！${NC}"
  echo -e "${GREEN}可以通过查看 logs/ 目录下的日志文件监控服务状态${NC}"
}

# 停止所有服务
stop_all() {
  echo -e "${GREEN}正在停止所有服务...${NC}"
  
  for service in "${SERVICES[@]}"; do
    if [ -f "logs/$service.pid" ]; then
      PID=$(cat "logs/$service.pid")
      if ps -p "$PID" > /dev/null; then
        echo -e "${GREEN}正在停止 $service (PID: $PID)...${NC}"
        kill "$PID"
        rm "logs/$service.pid"
      else
        echo -e "${RED}服务 $service 的PID文件存在，但进程不存在${NC}"
        rm "logs/$service.pid"
      fi
    else
      # 尝试通过进程名查找并终止
      PIDS=$(pgrep -f "target/release/$service")
      if [ -n "$PIDS" ]; then
        echo -e "${GREEN}正在停止 $service (PID: $PIDS)...${NC}"
        kill $PIDS
      else
        echo -e "${RED}服务 $service 未运行${NC}"
      fi
    fi
  done
  
  echo -e "${GREEN}所有服务已停止${NC}"
}

# 查看所有服务状态
status() {
  echo -e "${GREEN}检查所有服务状态...${NC}"
  
  for service in "${SERVICES[@]}"; do
    if check_service "$service"; then
      PID=$(pgrep -f "target/release/$service")
      echo -e "${GREEN}服务 $service 正在运行 (PID: $PID)${NC}"
    else
      echo -e "${RED}服务 $service 未运行${NC}"
    fi
  done
}

# 根据命令行参数执行不同操作
case "$1" in
  start)
    start_all
    ;;
  stop)
    stop_all
    ;;
  restart)
    stop_all
    sleep 2
    start_all
    ;;
  status)
    status
    ;;
  *)
    echo "用法: $0 {start|stop|restart|status}"
    exit 1
    ;;
esac

exit 0 