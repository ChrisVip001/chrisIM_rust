#!/bin/bash

# MinIO 启动脚本
# 暴露9000端口，允许局域网访问

# 设置MinIO的访问密钥（你可以修改这些值）
export MINIO_ROOT_USER="minioadmin"
export MINIO_ROOT_PASSWORD="minioadmin123"

# 获取本机IP地址（macOS）
LOCAL_IP=$(ifconfig | grep -E "inet.*broadcast" | awk '{print $2}' | head -1)

echo "Starting MinIO server..."
echo "Access Key: $MINIO_ROOT_USER"
echo "Secret Key: $MINIO_ROOT_PASSWORD"
echo "Console URL: http://$LOCAL_IP:9001"
echo "API URL: http://$LOCAL_IP:9000"
echo "Local Console URL: http://localhost:9001"
echo "Local API URL: http://localhost:9000"

# 启动MinIO服务器
# --address 0.0.0.0:9000 让服务监听所有网络接口，允许局域网访问
# --console-address 0.0.0.0:9001 设置控制台地址，也允许局域网访问
minio server ~/minio-data \
  --address 0.0.0.0:9000 \
  --console-address 0.0.0.0:9001 