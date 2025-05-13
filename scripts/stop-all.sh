#!/bin/bash

# 停止所有运行的微服务
pkill -f "auth-service" || true
pkill -f "user-service" || true
pkill -f "group-service" || true
pkill -f "friend-service" || true
pkill -f "private-message-server" || true
pkill -f "group-message-server" || true
pkill -f "message-gateway" || true
pkill -f "gateway-service" || true

echo "All services stopped." 