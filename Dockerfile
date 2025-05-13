FROM rust:1.75-slim-bullseye as builder

# 安装依赖
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    build-essential \
    git \
    librdkafka-dev \
    && rm -rf /var/lib/apt/lists/*

# 创建一个非root用户
RUN useradd -m -u 1000 -s /bin/bash rustim

# 设置工作目录
WORKDIR /app

# 复制Cargo文件
COPY Cargo.toml Cargo.lock ./
COPY common/Cargo.toml common/
COPY cache/Cargo.toml cache/
COPY auth-service/Cargo.toml auth-service/
COPY user-service/Cargo.toml user-service/
COPY group-service/Cargo.toml group-service/
COPY friend-service/Cargo.toml friend-service/
COPY oss/Cargo.toml oss/
COPY msg-server/Cargo.toml msg-server/
COPY msg-gateway/Cargo.toml msg-gateway/
COPY api-gateway/Cargo.toml gateway-service/

# 创建空源文件，触发依赖下载
RUN mkdir -p common/src cache/src auth-service/src user-service/src group-service/src \
    friend-service/src oss/src msg-server/src msg-gateway/src api-gateway/src \
    && touch common/src/lib.rs cache/src/lib.rs auth-service/src/main.rs user-service/src/main.rs \
    group-service/src/main.rs friend-service/src/main.rs oss/src/main.rs msg-server/src/main.rs \
    msg-gateway/src/main.rs api-gateway/src/main.rs

# 下载依赖（利用缓存层）
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch --locked

# 复制源代码
COPY common/ common/
COPY cache/ cache/
COPY auth-service/ auth-service/
COPY user-service/ user-service/
COPY group-service/ group-service/
COPY friend-service/ friend-service/
COPY oss/ oss/
COPY msg-server/ msg-server/
COPY msg-gateway/ msg-gateway/
COPY api-gateway/ gateway-service/
COPY config/ config/

# 构建所有服务（使用动态链接特性可以加速构建）
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --features dynamic --no-default-features

# 复制编译好的二进制文件到最终位置
RUN mkdir -p /app/bin \
    && cp target/release/auth-service /app/bin/ \
    && cp target/release/user-service /app/bin/ \
    && cp target/release/group-service /app/bin/ \
    && cp target/release/friend-service /app/bin/ \
    && cp target/release/oss /app/bin/ \
    && cp target/release/msg-server /app/bin/ \
    && cp target/release/msg-gateway /app/bin/ \
    && cp target/release/api-gateway /app/bin/

# 使用更小的基础镜像创建最终镜像
FROM debian:bullseye-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl1.1 \
    librdkafka1 \
    && rm -rf /var/lib/apt/lists/*

# 创建一个非root用户
RUN useradd -m -u 1000 -s /bin/bash rustim

# 复制编译好的二进制文件和配置
COPY --from=builder /app/bin /app/bin
COPY config/ /app/config/
COPY scripts/ /app/scripts/

# 设置工作目录
WORKDIR /app

# 设置环境变量
ENV PATH="/app/bin:${PATH}"

# 设置默认用户
USER rustim

# 容器启动脚本
COPY scripts/docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

ENTRYPOINT ["/app/docker-entrypoint.sh"]

# 默认启动网关服务
CMD ["gateway-service"] 