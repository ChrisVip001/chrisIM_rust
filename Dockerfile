# 多阶段构建 Dockerfile for RustIM - 优化版本
# 阶段1: 构建环境
FROM rust:1.85-slim-bullseye as builder

# 设置环境变量优化编译速度
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV RUSTFLAGS="-C target-cpu=native -C opt-level=2"
ENV CARGO_BUILD_JOBS=8
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

# 使用国内镜像源加速
RUN sed -i 's/deb.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list && \
    sed -i 's/security.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list

# 一次性安装所有构建依赖
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
    && rm -rf /var/lib/apt/lists/*

# 配置 Cargo 使用国内镜像源
RUN mkdir -p /usr/local/cargo && \
    echo '[source.crates-io]' > /usr/local/cargo/config.toml && \
    echo 'replace-with = "rsproxy-sparse"' >> /usr/local/cargo/config.toml && \
    echo '' >> /usr/local/cargo/config.toml && \
    echo '[source.rsproxy-sparse]' >> /usr/local/cargo/config.toml && \
    echo 'registry = "sparse+https://rsproxy.cn/index/"' >> /usr/local/cargo/config.toml && \
    echo '' >> /usr/local/cargo/config.toml && \
    echo '[registries.rsproxy-sparse]' >> /usr/local/cargo/config.toml && \
    echo 'index = "sparse+https://rsproxy.cn/index/"' >> /usr/local/cargo/config.toml && \
    echo '' >> /usr/local/cargo/config.toml && \
    echo '[build]' >> /usr/local/cargo/config.toml && \
    echo 'jobs = 8' >> /usr/local/cargo/config.toml && \
    echo '' >> /usr/local/cargo/config.toml && \
    echo '[profile.release]' >> /usr/local/cargo/config.toml && \
    echo 'opt-level = 2' >> /usr/local/cargo/config.toml && \
    echo 'lto = "thin"' >> /usr/local/cargo/config.toml && \
    echo 'codegen-units = 1' >> /usr/local/cargo/config.toml

WORKDIR /app

# 复制 Cargo 配置文件 - 优化缓存层
COPY Cargo.toml Cargo.lock ./
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

# 复制 proto 文件（protobuf 编译需要）
COPY common/proto/ common/proto/
COPY common/build.rs common/

# 创建虚拟源文件进行依赖预构建
RUN mkdir -p common/src cache/src user-service/src group-service/src friend-service/src \
    oss/src msg-server/src msg-gateway/src api-gateway/src msg-storage/src && \
    echo "fn main() {}" > user-service/src/main.rs && \
    echo "fn main() {}" > group-service/src/main.rs && \
    echo "fn main() {}" > friend-service/src/main.rs && \
    echo "fn main() {}" > oss/src/main.rs && \
    echo "fn main() {}" > msg-server/src/main.rs && \
    echo "fn main() {}" > msg-gateway/src/main.rs && \
    echo "fn main() {}" > api-gateway/src/main.rs && \
    echo "fn main() {}" > msg-storage/src/main.rs && \
    echo "// lib" > common/src/lib.rs && \
    echo "// lib" > cache/src/lib.rs

# 预构建依赖 - 使用缓存挂载
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --jobs 8

# 删除虚拟源文件
RUN rm -rf */src

# 复制实际源代码
COPY common/src/ common/src/
COPY cache/src/ cache/src/
COPY user-service/src/ user-service/src/
COPY group-service/src/ group-service/src/
COPY friend-service/src/ friend-service/src/
COPY oss/src/ oss/src/
COPY msg-server/src/ msg-server/src/
COPY msg-gateway/src/ msg-gateway/src/
COPY api-gateway/src/ api-gateway/src/
COPY msg-storage/src/ msg-storage/src/

# 最终构建
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --jobs 8 && \
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

# 使用国内镜像源
RUN sed -i 's/deb.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list && \
    sed -i 's/security.debian.org/mirrors.aliyun.com/g' /etc/apt/sources.list

# 安装运行时依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl1.1 \
    librdkafka1 \
    libpq5 \
    curl \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN groupadd -r rustim && useradd -r -g rustim -u 1000 rustim

WORKDIR /app

# 复制二进制文件
COPY --from=builder /app/bin /app/bin

# 复制配置文件和脚本
COPY config/ /app/config/
COPY scripts/docker-entrypoint.sh /app/docker-entrypoint.sh

# 设置权限
RUN chmod +x /app/docker-entrypoint.sh /app/bin/* && \
    chown -R rustim:rustim /app

# 设置环境变量
ENV PATH="/app/bin:${PATH}" \
    RUST_LOG=info \
    RUST_BACKTRACE=1

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