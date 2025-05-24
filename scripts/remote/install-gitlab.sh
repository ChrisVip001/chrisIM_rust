#!/bin/bash

set -e

GITLAB_IP="43.139.231.100"
GITLAB_PORT_HTTP=8080
GITLAB_PORT_SSH=2222
INSTALL_DIR="$HOME/gitlab"

echo "🚀 安装 Docker..."
if ! command -v docker &>/dev/null; then
  curl -fsSL https://get.docker.com | bash
else
  echo "✅ Docker 已安装"
fi

echo "🚀 安装 Docker Compose..."
if ! docker compose version &>/dev/null; then
  mkdir -p /usr/local/lib/docker/cli-plugins
  curl -SL https://github.com/docker/compose/releases/download/v2.24.6/docker-compose-linux-x86_64 -o /usr/local/lib/docker/cli-plugins/docker-compose
  chmod +x /usr/local/lib/docker/cli-plugins/docker-compose
else
  echo "✅ Docker Compose 已安装"
fi

echo "📁 创建目录结构..."
mkdir -p "$INSTALL_DIR/config/gitlab/config"
mkdir -p "$INSTALL_DIR/config/gitlab/logs"
mkdir -p "$INSTALL_DIR/config/gitlab/data"

echo "📄 生成 docker-compose.yml..."
cat > "$INSTALL_DIR/docker-compose.yml" <<EOF
version: '3.8'

services:
  gitlab:
    image: gitlab/gitlab-ce:latest
    container_name: gitlab
    restart: always
    hostname: $GITLAB_IP
    environment:
      GITLAB_OMNIBUS_CONFIG: |
        external_url 'http://$GITLAB_IP:$GITLAB_PORT_HTTP'
        gitlab_rails['gitlab_ssh_host'] = '$GITLAB_IP'
        gitlab_rails['gitlab_shell_ssh_port'] = $GITLAB_PORT_SSH
    ports:
      - "$GITLAB_PORT_HTTP:80"
      - "$GITLAB_PORT_SSH:22"
    volumes:
      - ./config/gitlab/config:/etc/gitlab
      - ./config/gitlab/logs:/var/log/gitlab
      - ./config/gitlab/data:/var/opt/gitlab
EOF

echo "🚀 启动 GitLab 服务..."
cd "$INSTALL_DIR"
docker compose up -d

echo ""
echo "✅ GitLab 部署完成！"
echo "🌐 打开浏览器访问：http://$GITLAB_IP:$GITLAB_PORT_HTTP"
echo "🔐 获取初始 root 密码："
echo "    docker exec -it gitlab cat /etc/gitlab/initial_root_password"