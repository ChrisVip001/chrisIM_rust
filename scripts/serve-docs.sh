#!/bin/bash

# 确保文档已经生成
./scripts/generate-docs.sh

# 创建文档首页
cat > docs/api/home.html << EOF
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>RustIM API 文档</title>
    <style>
        body {
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            margin: 0;
            padding: 0;
            color: #333;
            background-color: #f8f9fa;
        }
        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
        }
        header {
            background-color: #343a40;
            color: white;
            padding: 1rem;
            text-align: center;
        }
        h1 {
            margin: 0;
            font-size: 2rem;
        }
        .api-section {
            background-color: white;
            border-radius: 5px;
            box-shadow: 0 2px 5px rgba(0,0,0,0.1);
            margin: 20px 0;
            padding: 20px;
        }
        .api-section h2 {
            margin-top: 0;
            color: #007bff;
            border-bottom: 1px solid #eee;
            padding-bottom: 10px;
        }
        .doc-links {
            display: flex;
            flex-wrap: wrap;
            gap: 20px;
            margin-top: 20px;
        }
        .doc-link {
            display: block;
            background-color: #007bff;
            color: white;
            padding: 15px 25px;
            border-radius: 5px;
            text-decoration: none;
            transition: background-color 0.3s;
            font-weight: bold;
            min-width: 200px;
            text-align: center;
        }
        .doc-link:hover {
            background-color: #0056b3;
        }
        .service-list {
            margin-top: 20px;
        }
        .service-list div {
            background-color: #f8f9fa;
            padding: 10px 15px;
            margin-bottom: 10px;
            border-radius: 4px;
            border-left: 4px solid #007bff;
        }
    </style>
</head>
<body>
    <header>
        <h1>RustIM API 文档</h1>
    </header>
    <div class="container">
        <div class="api-section">
            <h2>接口文档导航</h2>
            <p>欢迎使用 RustIM API 文档。这里提供了系统中所有服务的接口文档，包括 gRPC 接口和 REST API。</p>
            
            <div class="doc-links">
                <a class="doc-link" href="index.html">gRPC 接口文档</a>
                <a class="doc-link" href="http://localhost:8000/swagger-ui/">REST API 文档</a>
            </div>
        </div>
        
        <div class="api-section">
            <h2>服务列表</h2>
            <div class="service-list">
                <div>用户服务 (user-service) - 处理用户注册、登录和信息管理</div>
                <div>好友服务 (friend-service) - 管理用户好友关系</div>
                <div>群组服务 (group-service) - 管理群组和成员关系</div>
                <div>认证服务 (auth-service) - 处理用户认证和授权</div>
                <div>消息网关 (msg-gateway) - 处理消息路由和分发</div>
            </div>
        </div>
        
        <div class="api-section">
            <h2>使用说明</h2>
            <p><strong>gRPC 接口文档：</strong> 提供所有微服务的 gRPC 接口定义，包括请求、响应和错误状态码。</p>
            <p><strong>REST API 文档：</strong> 提供 API 网关提供的 REST 接口，通过 Swagger UI 展示，支持在线调试。</p>
            <p>要在线调试 REST API，请确保已启动 API 网关服务。</p>
        </div>
    </div>
</body>
</html>
EOF

# 启动一个简单的HTTP服务器提供文档
echo "启动文档服务器，访问 http://localhost:8080/home.html 查看文档"
cd docs/api && python3 -m http.server 8080 