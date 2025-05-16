#!/bin/bash

# 创建文档目录
mkdir -p docs/api

# 使用Docker运行protoc-gen-doc为所有proto文件生成文档
docker run --rm \
  -v $(pwd)/common/proto:/protos \
  -v $(pwd)/docs/api:/out \
  pseudomuto/protoc-gen-doc \
  --doc_opt=html,index.html \
  *.proto

echo "gRPC API文档已生成到 docs/api/index.html"

# 确保文档目录有合适的权限
chmod -R 755 docs/api 