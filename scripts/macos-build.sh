#!/bin/bash
# MacOS环境下的构建脚本

echo "检查MacOS环境下构建所需的依赖..."

# 检查是否安装了Homebrew
if ! command -v brew &> /dev/null; then
    echo "错误: 未找到Homebrew，请先安装Homebrew"
    echo "安装命令: /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
    exit 1
fi

# 检查并安装必要的依赖
DEPS=("cmake" "pkg-config" "openssl" "librdkafka")
MISSING_DEPS=()

for dep in "${DEPS[@]}"; do
    if ! brew list $dep &> /dev/null; then
        MISSING_DEPS+=($dep)
    fi
done

if [ ${#MISSING_DEPS[@]} -ne 0 ]; then
    echo "需要安装以下依赖: ${MISSING_DEPS[*]}"
    read -p "是否立即安装? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        brew install ${MISSING_DEPS[*]}
    else
        echo "请手动安装所需依赖后再次运行此脚本"
        exit 1
    fi
fi

echo "所有依赖已满足，准备构建..."

# 设置环境变量
export OPENSSL_DIR=$(brew --prefix openssl)
export LIBRDKAFKA_DIR=$(brew --prefix librdkafka)

# 询问构建方式
echo "选择构建方式:"
echo "1. 使用系统librdkafka (推荐，更快速)"
echo "2. 使用内置cmake构建 (更稳定，但构建时间更长)"
read -p "请选择 [1/2]: " BUILD_CHOICE

if [ "$BUILD_CHOICE" = "1" ]; then
    echo "使用系统librdkafka构建..."
    cargo build --features dynamic --no-default-features
else
    echo "使用内置cmake构建..."
    cargo build
fi

if [ $? -ne 0 ]; then
    echo "构建失败，可能原因:"
    echo "1. 可能需要更新Rust工具链: rustup update"
    echo "2. 尝试使用内置cmake构建: cargo build --features static --no-default-features"
    echo "3. 检查您的Homebrew依赖是否正确安装: brew doctor"
else
    echo "构建成功!"
fi 