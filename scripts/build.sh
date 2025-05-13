#!/bin/bash
# 通用构建脚本 - 自动检测环境并调用相应的构建脚本

set -e

# 脚本目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 检测操作系统
detect_os() {
    case "$(uname -s)" in
        Darwin*)    
            echo "检测到 macOS 环境"
            echo "调用 macOS 构建脚本..."
            chmod +x "${SCRIPT_DIR}/macos-build.sh"
            "${SCRIPT_DIR}/macos-build.sh"
            ;;
        Linux*)     
            echo "检测到 Linux 环境"
            echo "调用 Linux 构建脚本..."
            chmod +x "${SCRIPT_DIR}/linux-build.sh"
            "${SCRIPT_DIR}/linux-build.sh"
            ;;
        CYGWIN*|MINGW*|MSYS*|Windows*)
            echo "检测到 Windows 环境"
            echo "请运行 windows-build.bat 脚本"
            exit 1
            ;;
        *)          
            echo "未知操作系统: $(uname -s)"
            echo "请手动选择构建脚本:"
            echo "1. MacOS (macos-build.sh)"
            echo "2. Linux (linux-build.sh)"
            echo "3. Windows (windows-build.bat)"
            read -p "选择 [1-3]: " os_choice
            case $os_choice in
                1)
                    chmod +x "${SCRIPT_DIR}/macos-build.sh"
                    "${SCRIPT_DIR}/macos-build.sh"
                    ;;
                2)
                    chmod +x "${SCRIPT_DIR}/linux-build.sh"
                    "${SCRIPT_DIR}/linux-build.sh"
                    ;;
                3)
                    echo "请运行 windows-build.bat 脚本"
                    exit 1
                    ;;
                *)
                    echo "无效的选择"
                    exit 1
                    ;;
            esac
            ;;
    esac
}

# 主函数
main() {
    echo "RustIM 跨平台构建工具"
    echo "======================="
    detect_os
}

main "$@" 