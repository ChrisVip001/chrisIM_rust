#!/bin/bash

# OpenCloudOS 用户管理脚本

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

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
    echo "OpenCloudOS 用户管理脚本"
    echo ""
    echo "用法: $0 [选项]"
    echo ""
    echo "选项:"
    echo "  list                 列出所有用户"
    echo "  list-normal          列出普通用户"
    echo "  list-logged          列出当前登录用户"
    echo "  create <username>    创建新用户"
    echo "  create-admin <user>  创建管理员用户"
    echo "  delete <username>    删除用户"
    echo "  info <username>      查看用户信息"
    echo "  passwd <username>    修改用户密码"
    echo "  help                 显示此帮助信息"
    echo ""
}

# 列出所有用户
list_users() {
    log_info "系统中的所有用户:"
    echo "用户名:UID:主目录:Shell"
    echo "------------------------"
    awk -F: '{printf "%-15s %-6s %-20s %s\n", $1, $3, $6, $7}' /etc/passwd
}

# 列出普通用户
list_normal_users() {
    log_info "普通用户 (UID >= 1000):"
    echo "用户名:UID:主目录:Shell"
    echo "------------------------"
    awk -F: '$3 >= 1000 {printf "%-15s %-6s %-20s %s\n", $1, $3, $6, $7}' /etc/passwd
}

# 列出当前登录用户
list_logged_users() {
    log_info "当前登录用户:"
    w
}

# 创建普通用户
create_user() {
    local username=$1
    
    if [[ -z "$username" ]]; then
        log_error "请提供用户名"
        exit 1
    fi
    
    # 检查用户是否已存在
    if id "$username" &>/dev/null; then
        log_error "用户 $username 已存在"
        exit 1
    fi
    
    log_info "创建用户: $username"
    
    # 创建用户
    sudo useradd -m -s /bin/bash -c "Created by user-management script" "$username"
    
    # 设置密码
    log_info "请为用户 $username 设置密码:"
    sudo passwd "$username"
    
    # 添加到 docker 组（如果存在）
    if getent group docker > /dev/null 2>&1; then
        sudo usermod -aG docker "$username"
        log_info "已将用户添加到 docker 组"
    fi
    
    log_success "用户 $username 创建成功"
    
    # 显示用户信息
    id "$username"
}

# 创建管理员用户
create_admin_user() {
    local username=$1
    
    if [[ -z "$username" ]]; then
        log_error "请提供用户名"
        exit 1
    fi
    
    # 检查用户是否已存在
    if id "$username" &>/dev/null; then
        log_error "用户 $username 已存在"
        exit 1
    fi
    
    log_info "创建管理员用户: $username"
    
    # 创建用户并添加到 wheel 组
    sudo useradd -m -s /bin/bash -G wheel -c "Admin user created by script" "$username"
    
    # 设置密码
    log_info "请为管理员用户 $username 设置密码:"
    sudo passwd "$username"
    
    # 添加到 docker 组（如果存在）
    if getent group docker > /dev/null 2>&1; then
        sudo usermod -aG docker "$username"
        log_info "已将用户添加到 docker 组"
    fi
    
    log_success "管理员用户 $username 创建成功"
    log_warning "该用户具有 sudo 权限，请妥善保管密码"
    
    # 显示用户信息
    id "$username"
}

# 删除用户
delete_user() {
    local username=$1
    
    if [[ -z "$username" ]]; then
        log_error "请提供用户名"
        exit 1
    fi
    
    # 检查用户是否存在
    if ! id "$username" &>/dev/null; then
        log_error "用户 $username 不存在"
        exit 1
    fi
    
    # 确认删除
    read -p "确定要删除用户 $username 吗？这将删除用户主目录 (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        log_info "取消删除操作"
        exit 0
    fi
    
    log_info "删除用户: $username"
    sudo userdel -r "$username"
    log_success "用户 $username 已删除"
}

# 查看用户信息
show_user_info() {
    local username=$1
    
    if [[ -z "$username" ]]; then
        log_error "请提供用户名"
        exit 1
    fi
    
    # 检查用户是否存在
    if ! id "$username" &>/dev/null; then
        log_error "用户 $username 不存在"
        exit 1
    fi
    
    log_info "用户 $username 的详细信息:"
    echo ""
    echo "基本信息:"
    id "$username"
    echo ""
    echo "用户详情:"
    getent passwd "$username"
    echo ""
    echo "所属组:"
    groups "$username"
    echo ""
    echo "最近登录:"
    last "$username" | head -5
}

# 修改用户密码
change_password() {
    local username=$1
    
    if [[ -z "$username" ]]; then
        log_error "请提供用户名"
        exit 1
    fi
    
    # 检查用户是否存在
    if ! id "$username" &>/dev/null; then
        log_error "用户 $username 不存在"
        exit 1
    fi
    
    log_info "修改用户 $username 的密码:"
    sudo passwd "$username"
}

# 主函数
main() {
    case "${1:-help}" in
        "list")
            list_users
            ;;
        "list-normal")
            list_normal_users
            ;;
        "list-logged")
            list_logged_users
            ;;
        "create")
            create_user "$2"
            ;;
        "create-admin")
            create_admin_user "$2"
            ;;
        "delete")
            delete_user "$2"
            ;;
        "info")
            show_user_info "$2"
            ;;
        "passwd")
            change_password "$2"
            ;;
        "help"|*)
            show_help
            ;;
    esac
}

# 检查是否有 sudo 权限
if [[ $EUID -ne 0 ]] && ! sudo -n true 2>/dev/null; then
    log_info "此脚本需要 sudo 权限"
    sudo -v
fi

# 执行主函数
main "$@" 