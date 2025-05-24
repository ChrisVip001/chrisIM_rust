# 多阶段构建 Dockerfile for RustIM
# 阶段1: 构建环境
FROM rust:1.75-slim-bullseye as builder

# 设置环境变量
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV RUSTFLAGS="-C target-cpu=native"

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    build-essential \
    git \
    librdkafka-dev \
    libpq-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# 创建应用目录
WORKDIR /app

# 复制 Cargo 配置文件
COPY Cargo.toml Cargo.lock ./

# 复制所有服务的 Cargo.toml 文件
COPY common/Cargo.toml common/
COPY cache/Cargo.toml cache/
COPY user-service/Cargo.toml user-service/
COPY group-service/Cargo.toml group-service/
COPY friend-service/Cargo.toml friend-service/
COPY oss/Cargo.toml oss/
COPY msg-server/Cargo.toml msg-server/
COPY msg-gateway/Cargo.toml msg-gateway/
COPY api-gateway/Cargo.toml api-gateway/
COPY msg-storage/Cargo.toml msg-storage/

# 创建虚拟源文件以触发依赖下载
RUN mkdir -p \
    common/src \
    cache/src \
    user-service/src \
    group-service/src \
    friend-service/src \
    oss/src \
    msg-server/src \
    msg-gateway/src \
    api-gateway/src \
    msg-storage/src \
    && echo "fn main() {}" > user-service/src/main.rs \
    && echo "fn main() {}" > group-service/src/main.rs \
    && echo "fn main() {}" > friend-service/src/main.rs \
    && echo "fn main() {}" > oss/src/main.rs \
    && echo "fn main() {}" > msg-server/src/main.rs \
    && echo "fn main() {}" > msg-gateway/src/main.rs \
    && echo "fn main() {}" > api-gateway/src/main.rs \
    && echo "fn main() {}" > msg-storage/src/main.rs \
    && echo "// lib" > common/src/lib.rs \
    && echo "// lib" > cache/src/lib.rs

# 预构建依赖（利用 Docker 缓存层）
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release

# 删除虚拟源文件
RUN rm -rf \
    common/src \
    cache/src \
    user-service/src \
    group-service/src \
    friend-service/src \
    oss/src \
    msg-server/src \
    msg-gateway/src \
    api-gateway/src \
    msg-storage/src

# 复制实际源代码
COPY common/ common/
COPY cache/ cache/
COPY user-service/ user-service/
COPY group-service/ group-service/
COPY friend-service/ friend-service/
COPY oss/ oss/
COPY msg-server/ msg-server/
COPY msg-gateway/ msg-gateway/
COPY api-gateway/ api-gateway/
COPY msg-storage/ msg-storage/

# 构建所有服务
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    mkdir -p /app/bin && \
    cp target/release/user-service /app/bin/ && \
    cp target/release/group-service /app/bin/ && \
    cp target/release/friend-service /app/bin/ && \
    cp target/release/oss /app/bin/ && \
    cp target/release/msg-server /app/bin/ && \
    cp target/release/msg-gateway /app/bin/ && \
    cp target/release/api-gateway /app/bin/ && \
    strip /app/bin/*

# 阶段2: 运行时环境
FROM debian:bullseye-slim as runtime

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl1.1 \
    librdkafka1 \
    libpq5 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN groupadd -r rustim && useradd -r -g rustim -u 1000 rustim

# 创建应用目录
WORKDIR /app

# 复制二进制文件
COPY --from=builder /app/bin /app/bin

# 复制配置文件和脚本
COPY config/ /app/config/
COPY scripts/docker-entrypoint.sh /app/docker-entrypoint.sh

# 设置权限
RUN chmod +x /app/docker-entrypoint.sh && \
    chmod +x /app/bin/* && \
    chown -R rustim:rustim /app

# 设置环境变量
ENV PATH="/app/bin:${PATH}"
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# 健康检查
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# 暴露端口
EXPOSE 8080 8085 50001 50002 50003 50004 50005

# 切换到非 root 用户
USER rustim

# 设置入口点
ENTRYPOINT ["/app/docker-entrypoint.sh"]

# 默认启动 API 网关
CMD ["api-gateway"]

# 阶段3: 开发环境 (可选)
FROM builder as development

# 安装开发工具
RUN cargo install cargo-watch cargo-audit

# 复制源代码
COPY . .

# 暴露端口
EXPOSE 8080 8085 50001 50002 50003 50004 50005

# 开发模式入口点
CMD ["cargo", "run", "--bin", "api-gateway"]

# 阶段4: 生产环境 (最小化镜像)
FROM gcr.io/distroless/cc-debian11 as production

# 复制二进制文件
COPY --from=builder /app/bin /app/bin

# 复制配置文件
COPY config/ /app/config/

# 设置工作目录
WORKDIR /app

# 设置环境变量
ENV PATH="/app/bin:${PATH}"

# 暴露端口
EXPOSE 8080

# 使用非 root 用户
USER 1000

# 启动应用
ENTRYPOINT ["/app/bin/api-gateway"] 