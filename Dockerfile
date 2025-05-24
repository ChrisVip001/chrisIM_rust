# 多阶段构建 Dockerfile for RustIM
# 阶段1: 构建环境
FROM rust:1.76-slim-bullseye as builder

# 设置环境变量
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV RUSTFLAGS="-C target-cpu=native"

# 使用国内镜像源加速（可选，根据服务器位置调整）
RUN if [ -f /etc/apt/sources.list ]; then \
        cp /etc/apt/sources.list /etc/apt/sources.list.bak && \
        sed -i 's/deb.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list && \
        sed -i 's/security.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list; \
    fi

# 安装构建依赖 - 优化包管理器缓存
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    pkg-config \
    libssl-dev \
    build-essential \
    git \
    librdkafka-dev \
    libpq-dev \
    protobuf-compiler \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*

# 配置 Cargo 使用国内镜像源
RUN mkdir -p /usr/local/cargo && \
    echo '[source.crates-io]' > /usr/local/cargo/config.toml && \
    echo 'replace-with = "rsproxy-sparse"' >> /usr/local/cargo/config.toml && \
    echo '[source.rsproxy-sparse]' >> /usr/local/cargo/config.toml && \
    echo 'registry = "sparse+https://rsproxy.cn/index/"' >> /usr/local/cargo/config.toml && \
    echo '[registries.rsproxy-sparse]' >> /usr/local/cargo/config.toml && \
    echo 'index = "sparse+https://rsproxy.cn/index/"' >> /usr/local/cargo/config.toml

# 创建应用目录
WORKDIR /app

# 显示构建上下文信息（调试用）
RUN echo "=== Docker 构建开始 ===" && \
    echo "工作目录: $(pwd)" && \
    echo "用户: $(whoami)" && \
    echo "构建时间: $(date)" && \
    echo "Rust 版本: $(rustc --version)" && \
    echo "Cargo 版本: $(cargo --version)"

# 复制 Cargo 配置文件
COPY Cargo.toml ./

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

# 处理 Cargo.lock 文件 - 改进版本兼容性处理
RUN echo "=== 处理 Cargo.lock 文件 ===" && \
    if [ -f "Cargo.lock" ]; then \
        echo "检测到现有 Cargo.lock 文件" && \
        # 检查 Cargo.lock 版本兼容性 \
        if cargo tree --version > /dev/null 2>&1; then \
            echo "Cargo.lock 版本兼容，使用现有文件" && \
            ls -la Cargo.lock; \
        else \
            echo "Cargo.lock 版本不兼容，重新生成" && \
            rm -f Cargo.lock && \
            cargo generate-lockfile && \
            echo "重新生成 Cargo.lock 完成"; \
        fi; \
    else \
        echo "Cargo.lock 不存在，正在生成..." && \
        cargo generate-lockfile && \
        echo "Cargo.lock 生成完成"; \
    fi

# 尝试复制 Cargo.lock 文件（如果存在且兼容）
COPY Cargo.loc[k] ./

# 最终验证和生成 Cargo.lock
RUN echo "=== 最终验证 Cargo.lock ===" && \
    if [ ! -f "Cargo.lock" ]; then \
        echo "生成新的 Cargo.lock 文件..." && \
        cargo generate-lockfile; \
    fi && \
    echo "验证 Cargo.lock 兼容性..." && \
    if ! cargo tree --version > /dev/null 2>&1; then \
        echo "Cargo.lock 不兼容，重新生成..." && \
        rm -f Cargo.lock && \
        cargo generate-lockfile; \
    fi && \
    echo "=== Cargo.lock 处理完成 ===" && \
    ls -la Cargo.toml Cargo.lock && \
    echo "Cargo.toml 内容预览:" && \
    head -10 Cargo.toml && \
    echo "Cargo.lock 文件大小: $(wc -l < Cargo.lock) 行" && \
    echo "Cargo.lock 版本: $(head -3 Cargo.lock | grep version || echo '未知版本')"

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
    --mount=type=cache,target=/app/target \
    echo "=== 开始预构建依赖 ===" && \
    echo "当前 Cargo.lock 状态:" && \
    ls -la Cargo.lock && \
    echo "开始构建..." && \
    cargo build --release && \
    echo "=== 依赖预构建完成 ==="

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
    echo "=== 开始构建应用 ===" && \
    cargo build --release && \
    echo "=== 应用构建完成 ===" && \
    mkdir -p /app/bin && \
    cp target/release/user-service /app/bin/ && \
    cp target/release/group-service /app/bin/ && \
    cp target/release/friend-service /app/bin/ && \
    cp target/release/oss /app/bin/ && \
    cp target/release/msg-server /app/bin/ && \
    cp target/release/msg-gateway /app/bin/ && \
    cp target/release/api-gateway /app/bin/ && \
    strip /app/bin/* && \
    echo "=== 二进制文件准备完成 ==="

# 阶段2: 运行时环境
FROM debian:bullseye-slim as runtime

# 使用国内镜像源加速（可选）
RUN if [ -f /etc/apt/sources.list ]; then \
        cp /etc/apt/sources.list /etc/apt/sources.list.bak && \
        sed -i 's/deb.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list && \
        sed -i 's/security.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list; \
    fi

# 安装运行时依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl1.1 \
    librdkafka1 \
    libpq5 \
    curl \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*

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