.PHONY: start stop restart status install uninstall

# 默认目标：显示帮助信息
help:
	@echo "使用方法:"
	@echo "  make start    - 启动所有服务"
	@echo "  make stop     - 停止所有服务"
	@echo "  make restart  - 重启所有服务"
	@echo "  make status   - 查看服务状态"
	@echo "  make install  - 安装为系统服务 (macOS)"
	@echo "  make uninstall- 卸载系统服务 (macOS)"

# 启动所有服务
start:
	@./scripts/start_all_services.sh start

# 停止所有服务
stop:
	@./scripts/start_all_services.sh stop

# 重启所有服务
restart:
	@./scripts/start_all_services.sh restart

# 查看服务状态
status:
	@./scripts/start_all_services.sh status

# 安装为系统服务 (macOS)
install:
	@./scripts/install_system_service.sh

# 卸载系统服务 (macOS)
uninstall:
	@launchctl unload ~/Library/LaunchAgents/com.rust-im.services.plist 2>/dev/null || true
	@rm -f ~/Library/LaunchAgents/com.rust-im.services.plist
	@echo "系统服务已卸载" 