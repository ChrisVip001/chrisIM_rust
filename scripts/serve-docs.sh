#!/bin/bash

# 确保文档目录存在
mkdir -p docs/api

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
                <a class="doc-link" href="http://localhost:8000/api-doc/openapi.json">REST API 文档</a>
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
            <p><strong>REST API 文档：</strong> 提供 API 网关提供的 OpenAPI 格式的 REST 接口定义。</p>
            <p>要查看完整的 REST API 文档，请确保已启动 API 网关服务：<code>cargo run -p api-gateway</code></p>
        </div>
    </div>
</body>
</html>
EOF

# 如果index.html不存在，创建一个简单的示例
if [ ! -f docs/api/index.html ]; then
    cat > docs/api/index.html << EOF
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>RustIM gRPC API 文档</title>
    <style>
        body {
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            margin: 0;
            padding: 20px;
            color: #333;
        }
        h1 {
            color: #007bff;
            border-bottom: 1px solid #eee;
            padding-bottom: 10px;
        }
        .note {
            background-color: #f8d7da;
            border: 1px solid #f5c6cb;
            color: #721c24;
            padding: 15px;
            border-radius: 5px;
            margin: 20px 0;
        }
        .service {
            background-color: #f8f9fa;
            border-radius: 5px;
            padding: 20px;
            margin: 20px 0;
            border-left: 4px solid #007bff;
        }
        h2 {
            color: #0056b3;
            margin-top: 0;
        }
        a {
            color: #007bff;
            text-decoration: none;
        }
        a:hover {
            text-decoration: underline;
        }
    </style>
</head>
<body>
    <h1>RustIM gRPC API 文档</h1>
    
    <div class="note">
        <p><strong>注意：</strong> 这是一个占位文档。要生成完整的 gRPC API 文档，请确保 Docker 守护程序正在运行，然后执行 <code>./scripts/generate-docs.sh</code> 脚本。</p>
        <p>您需要安装 Docker 并确保其运行状态正常。</p>
    </div>
    
    <div class="service">
        <h2>用户服务</h2>
        <p>用户服务提供用户管理功能，包括用户注册、登录、信息查询等。</p>
        <p>主要接口包括：</p>
        <ul>
            <li>CreateUser：创建新用户</li>
            <li>GetUserById：根据ID获取用户信息</li>
            <li>GetUserByUsername：根据用户名获取用户信息</li>
            <li>UpdateUser：更新用户信息</li>
            <li>VerifyPassword：验证用户密码</li>
            <li>SearchUsers：搜索用户</li>
        </ul>
    </div>
    
    <div class="service">
        <h2>好友服务</h2>
        <p>好友服务管理用户之间的好友关系。</p>
        <p>主要接口包括：</p>
        <ul>
            <li>AddFriend：添加好友</li>
            <li>DeleteFriend：删除好友</li>
            <li>GetFriendList：获取好友列表</li>
        </ul>
    </div>
    
    <a href="home.html">返回文档首页</a>
</body>
</html>
EOF
fi

# 启动一个简单的HTTP服务器提供文档
echo "=========================================================="
echo "🚀 RustIM API 文档服务器已启动"
echo "--------------------------------------------------------"
echo "📚 文档首页：http://localhost:8080/home.html"
echo "📘 gRPC 接口文档：http://localhost:8080/index.html"
echo "📗 REST API 文档：http://localhost:8000/api-doc/openapi.json"
echo "   (需要先启动 API 网关：cargo run -p api-gateway)"
echo "=========================================================="
cd docs/api && python3 -m http.server 8080 