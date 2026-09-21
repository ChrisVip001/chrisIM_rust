 #!/bin/bash

 # 日志系统升级辅助脚本
 # 用于批量替换现有的tracing日志调用为增强版本

 echo "🔧 开始升级日志系统..."

 # 检查是否在项目根目录
 if [ ! -f "Cargo.toml" ]; then
     echo "❌ 请在项目根目录运行此脚本"
     exit 1
 fi

 # 创建备份
 backup_dir="./backup_$(date +%Y%m%d_%H%M%S)"
 echo "📦 创建备份到: $backup_dir"
 cp -r ./src "$backup_dir" 2>/dev/null || echo "⚠️  无src目录或备份失败"

 # 函数：替换错误日志
 replace_error_logs() {
     local file="$1"
     echo "🔄 处理文件: $file"

     # 替换简单的error!调用
     sed -i.bak 's/tracing::error!(/error_with_location!(/' "$file"
     sed -i.bak 's/error!(/error_with_location!(/' "$file"

     # 替换warn!调用
     sed -i.bak 's/tracing::warn!(/warn_with_location!(/' "$file"
     sed -i.bak 's/warn!(/warn_with_location!(/' "$file"

     # 清理备份文件
     rm -f "${file}.bak"
 }

 # 函数：添加导入语句
 add_imports() {
     local file="$1"

     # 检查是否已有common导入
     if ! grep -q "use common::" "$file" 2>/dev/null; then
         # 在文件开头添加导入
         echo "📥 添加导入到: $file"
         echo "use common::{error_with_location, warn_with_location, info_with_location, debug_with_location};" > temp_file
         echo "" >> temp_file
         cat "$file" >> temp_file
         mv temp_file "$file"
     fi
 }

 # 选择性升级模式
 echo "请选择升级模式："
 echo "1) 仅升级错误日志 (推荐，最安全)"
 echo "2) 升级错误和警告日志"
 echo "3) 升级所有日志类型"
 echo "4) 仅添加panic处理"
 echo "5) 退出"

 read -p "请输入选择 (1-5): " choice

 case $choice in
     1)
         echo "🎯 仅升级错误日志..."
         find ./src -name "*.rs" -type f | while read file; do
             if grep -q "tracing::error!\|error!" "$file" 2>/dev/null; then
                 add_imports "$file"
                 sed -i.bak 's/tracing::error!(/error_with_location!(/' "$file"
                 sed -i.bak 's/error!(/error_with_location!(/' "$file"
                 rm -f "${file}.bak"
                 echo "✅ 已升级: $file"
             fi
         done
         ;;
     2)
         echo "🎯 升级错误和警告日志..."
         find ./src -name "*.rs" -type f | while read file; do
             if grep -q "tracing::\(error\|warn\)!\|\(error\|warn\)!" "$file" 2>/dev/null; then
                 add_imports "$file"
                 replace_error_logs "$file"
                 echo "✅ 已升级: $file"
             fi
         done
         ;;
     3)
         echo "🎯 升级所有日志类型..."
         find ./src -name "*.rs" -type f | while read file; do
             if grep -q "tracing::\|log::" "$file" 2>/dev/null; then
                 add_imports "$file"
                 replace_error_logs "$file"
                 # 替换info和debug
                 sed -i.bak 's/tracing::info!(/info_with_location!(/' "$file"
                 sed -i.bak 's/info!(/info_with_location!(/' "$file"
                 sed -i.bak 's/tracing::debug!(/debug_with_location!(/' "$file"
                 sed -i.bak 's/debug!(/debug_with_location!(/' "$file"
                 rm -f "${file}.bak"
                 echo "✅ 已升级: $file"
             fi
         done
         ;;
     4)
         echo "🎯 仅添加panic处理..."
         # 查找main.rs文件
         main_files=$(find ./src -name "main.rs" -o -name "lib.rs")
         for main_file in $main_files; do
             if [ -f "$main_file" ]; then
                 echo "📝 在 $main_file 中添加panic处理..."
                 # 在main函数开始处添加panic处理
                 sed -i.bak '/fn main/,/{/ a\
     // 设置panic处理器，记录详细错误位置\
     common::logging::setup_panic_hook();
 ' "$main_file"
                 rm -f "${main_file}.bak"
                 echo "✅ 已添加panic处理到: $main_file"
             fi
         done
         ;;
     5)
         echo "👋 退出升级"
         exit 0
         ;;
     *)
         echo "❌ 无效选择"
         exit 1
         ;;
 esac

 # 检查编译
 echo "🔨 检查编译..."
 if cargo check; then
     echo "✅ 升级成功！代码编译通过"
     echo "📝 请检查升级后的代码，确保符合预期"
     echo "🗂️  备份文件位于: $backup_dir"
 else
     echo "❌ 编译失败，请检查代码"
     echo "💡 可以从备份恢复: cp -r $backup_dir/src ."
     exit 1
 fi

 echo "🎉 日志系统升级完成！"
 echo ""
 echo "📋 后续步骤："
 echo "   1. 运行 cargo test 确保测试通过"
 echo "   2. 检查关键路径的日志输出"
 echo "   3. 逐步在新代码中使用更多增强功能"
 echo "   4. 如有问题，从 $backup_dir 恢复"