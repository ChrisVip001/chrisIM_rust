.PHONY: start stop restart status build

# 默认目标：显示帮助信息
help:
	@echo "使用方法:"
	@echo "  make build    - 构建所有服务"
	@echo "  make start    - 启动所有服务"
	@echo "  make stop     - 停止所有服务"
	@echo "  make restart  - 重启所有服务"
	@echo "  make status   - 查看服务状态"

# 构建所有服务
build:
	@./scripts/build.sh release

# 启动所有服务
start:
	@./scripts/start-services.sh start

# 停止所有服务
stop:
	@./scripts/start-services.sh stop

# 重启所有服务
restart:
	@./scripts/start-services.sh restart

# 查看服务状态
status:
	@./scripts/start-services.sh status 