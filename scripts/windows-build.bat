@echo off
echo 设置Windows环境下构建rdkafka所需的环境变量...

:: 检查是否存在CMake
where cmake >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo 错误: 未找到CMake，请安装CMake并确保添加到PATH
    echo 下载地址: https://cmake.org/download/
    exit /b 1
)

:: 检查是否存在Git
where git >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo 错误: 未找到Git，请安装Git并确保添加到PATH
    echo 下载地址: https://gitforwindows.org/
    exit /b 1
)

echo 所有依赖已满足，设置环境变量...

:: 设置环境变量
set CARGO_NET_GIT_FETCH_WITH_CLI=true
set CMAKE_GENERATOR=Visual Studio 17 2022

echo 选择构建方式:
echo 1. 使用CMake静态构建 (推荐，但需要更多依赖)
echo 2. 使用动态链接 (更简单，但性能可能较差)
choice /c 12 /n /m "请选择 [1,2]: "

if errorlevel 2 (
    echo 使用动态链接构建...
    set ENABLE_DYNAMIC=1
    cargo build --features dynamic --no-default-features
) else (
    echo 使用CMake静态构建...
    cargo build
)

if %ERRORLEVEL% NEQ 0 (
    echo 构建失败，您可以尝试以下步骤:
    echo 1. 确保安装了最新版本的Visual Studio Build Tools
    echo 2. 尝试使用WSL (Windows Subsystem for Linux)
    echo 3. 或者使用项目提供的Docker环境
) else (
    echo 构建成功!
)

pause 