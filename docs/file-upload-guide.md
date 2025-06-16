# 文件上传功能设计指南

## 概述

本系统采用前端直传对象存储的架构，通过预签名URL机制实现安全的文件上传功能，有效减轻服务器压力。支持多种存储提供商（AWS S3、腾讯云COS等）和无token的注册头像上传。

## 架构设计

```
┌─────────────┐    1. 请求预签名URL    ┌─────────────┐
│   前端应用   │ ──────────────────→ │   后端API   │
│             │                     │             │
│             │ ←────────────────── │             │
└─────────────┘    2. 返回预签名URL   └─────────────┘
        │                                   │
        │ 3. 直接上传文件                    │ 4. 验证上传
        ↓                                   ↓
┌─────────────┐                     ┌─────────────┐
│  对象存储    │                     │   后端API   │
│ (S3/COS)    │                     │             │
└─────────────┘                     └─────────────┘
```

## API接口详细说明

### 1. 获取预签名URL接口

**接口地址：** `POST /api/files/presigned-url`

**认证要求：** 需要JWT Token

**请求参数：**
```json
{
  "file_name": "document.pdf",
  "content_type": "application/pdf",
  "file_size": 1048576
}
```

**参数说明：**
- `file_name`: 文件名（必填）
- `content_type`: MIME类型（必填）
- `file_size`: 文件大小，字节为单位（必填）

**响应示例：**
```json
{
  "key": "uploads/2025/05/28/780152ab-9cbb-4ef3-b2bd-9f0479845935.ceshi",
  "upload_url": "http://127.0.0.1:9000/uploads/2025/05/28/780152ab-9cbb-4ef3-b2bd-9f0479845935.ceshi?q-sign-algorithm=sha1&q-ak=minioadmin&q-sign-time=1748415593;1748419193&q-key-time=1748415593;1748419193&q-header-list=content-type&q-url-param-list=&q-signature=a3faff4ffe925b12e9bb84447c22205cee86ed40"
}
```

**响应字段说明：**
- `upload_url`: 预签名上传URL，前端直接PUT请求此URL
- `file_key`: 文件在存储中的唯一标识
- `expires_in`: URL有效期（秒）

**安全限制：**
- 文件大小限制：50MB
- 支持的文件类型：根据content_type验证
- URL有效期：15分钟

### 2. 验证上传接口

**接口地址：** `POST /api/files/validate-upload`

**认证要求：** 需要JWT Token

**请求参数：**
```json
{
  "key": "files/uuid-filename.pdf",
  "size": 1048576,
  "md5": "d41d8cd98f00b204e9800998ecf8427e"
}
```

**参数说明：**
- `key`: 文件标识（必填）
- `size`: 文件大小（必填）
- `md5`: 文件MD5哈希值（必填）

**响应示例：**
```json
{
  "status": "success",
  "key": "files/uuid-filename.pdf",
  "is_valid": true
}
```

**验证流程：**
1. 检查文件是否存在于存储中
2. 验证文件大小是否匹配
3. 验证MD5哈希值是否正确
4. 返回验证结果和访问URL

### 3. 注册头像上传接口

**接口地址：** `POST /api/files/register-avatar`

**认证要求：** 无需Token（白名单接口）

**请求参数：**
```json
{
  "content_type": "image/jpeg",
  "file_size": 204800
}
```

**参数说明：**
- `content_type`: 图片MIME类型（必填）
- `file_size`: 文件大小（必填）

**响应示例：**
```json
{
  "key": "avatars/register/9cd772c9-1518-4964-88f4-6326a5bc525b.png",
  "upload_url": "http://127.0.0.1:9000/avatars/register/9cd772c9-1518-4964-88f4-6326a5bc525b.png?q-sign-algorithm=sha1&q-ak=minioadmin&q-sign-time=1748413094;1748414894&q-key-time=1748413094;1748414894&q-header-list=content-type&q-url-param-list=&q-signature=2c625234d686504a577ab195ddcccd59411d5ca9"
}
```

**安全限制：**
- 文件大小限制：2MB
- 支持的图片格式：JPG、PNG、WEBP
- URL有效期：10分钟

### 4. 验证注册头像接口

**接口地址：** `POST /api/files/validate-register-avatar`

**认证要求：** 无需Token（白名单接口）

**请求参数：**
```json
{
  "key": "avatars/register/9cd772c9-1518-4964-88f4-6326a5bc525b.png",
  "size": 1048576,
  "md5": "d41d8cd98f00b204e9800998ecf8427e"
}
```

**参数说明：**
- `key`: 头像文件标识（必填）
- `size`: 文件大小（必填）
- `md5`: 文件MD5哈希值（必填）

**响应示例：**
```json
{
  "status": "success",
  "key": "avatars/register/9cd772c9-1518-4964-88f4-6326a5bc525b.png",
  "is_valid": true
}
```

## 安全注意事项

### 1. 文件类型验证
- 前端和后端都需要验证文件类型
- 使用MIME类型和文件扩展名双重验证
- 禁止可执行文件上传

### 2. 文件大小限制
- 普通文件：限制50MB以内
- 头像文件：限制2MB以内
- 在前端和后端都进行大小检查

### 3. MD5完整性验证
- 前端计算文件MD5哈希值
- 后端验证上传后的文件MD5
- 确保文件传输完整性

### 4. 预签名URL安全
- 设置合理的过期时间（10-15分钟）
- 限制HTTP方法（仅PUT）
- 包含文件类型限制

### 5. 访问控制
- 普通文件上传需要JWT认证
- 注册头像上传使用白名单机制
- 定期清理未完成的上传

这个架构设计确保了文件上传的安全性、可靠性和高性能，同时支持多种使用场景和存储提供商。 