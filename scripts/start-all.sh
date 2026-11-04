#!/bin/bash
set -e

# 确保脚本目录存在
mkdir -p scripts

# 创建日志目录
mkdir -p logs

# 启动所有服务
echo "Starting auth-service..."
cargo run --bin auth-service > logs/auth-service.log 2>&1 &

echo "Starting relation-service (user/friend/group)..."
cargo run --bin relation-service > logs/relation-service.log 2>&1 &

echo "Starting private-message-server..."
cargo run --bin private-message-server > logs/private-message-server.log 2>&1 &

echo "Starting group-message-server..."
cargo run --bin group-message-server > logs/group-message-server.log 2>&1 &

echo "Starting message-gateway..."
cargo run --bin message-gateway > logs/message-gateway.log 2>&1 &

echo "Starting gateway-service..."
cargo run --bin api-gateway > logs/api-gateway.log 2>&1 &

echo "All services started. Check logs directory for output." 