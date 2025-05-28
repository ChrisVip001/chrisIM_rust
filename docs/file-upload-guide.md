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
  "code": 200,
  "message": "success",
  "data": {
    "upload_url": "https://bucket.cos.region.myqcloud.com/files/uuid-filename.pdf?q-sign-algorithm=sha1&...",
    "file_key": "files/uuid-filename.pdf",
    "expires_in": 900
  }
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
  "file_key": "files/uuid-filename.pdf",
  "file_size": 1048576,
  "md5_hash": "d41d8cd98f00b204e9800998ecf8427e"
}
```

**参数说明：**
- `file_key`: 文件标识（必填）
- `file_size`: 文件大小（必填）
- `md5_hash`: 文件MD5哈希值（必填）

**响应示例：**
```json
{
  "code": 200,
  "message": "Upload validated successfully",
  "data": {
    "valid": true,
    "file_url": "https://your-domain.com/files/uuid-filename.pdf"
  }
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
  "code": 200,
  "message": "success",
  "data": {
    "upload_url": "https://avatars.cos.region.myqcloud.com/avatars/uuid.jpg?q-sign-algorithm=sha1&...",
    "avatar_key": "avatars/uuid.jpg",
    "expires_in": 600
  }
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
  "avatar_key": "avatars/uuid.jpg",
  "file_size": 204800,
  "md5_hash": "a1b2c3d4e5f6789012345678901234567"
}
```

**参数说明：**
- `avatar_key`: 头像文件标识（必填）
- `file_size`: 文件大小（必填）
- `md5_hash`: 文件MD5哈希值（必填）

**响应示例：**
```json
{
  "code": 200,
  "message": "Avatar upload validated successfully",
  "data": {
    "valid": true,
    "avatar_url": "https://your-domain.com/avatars/uuid.jpg"
  }
}
```


## 网关白名单配置

在`config.yaml`中添加注册头像上传接口到白名单：

```yaml
gateway:
  path_whitelist:
    - /api/auth/login
    - /api/auth/register
    - /api/files/register-avatar          # 注册头像上传
    - /api/files/validate-register-avatar # 验证注册头像
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


## 性能优化建议

### 1. 前端优化
- 大文件分片上传
- 上传进度显示
- 断点续传支持
- 并发上传限制

### 2. 后端优化
- 预签名URL缓存
- 异步文件验证
- 批量操作支持
- 监控和日志

### 3. 存储优化
- CDN加速
- 多地域部署
- 生命周期管理
- 成本优化策略

## 监控和日志

### 关键指标
- 上传成功率
- 平均上传时间
- 文件大小分布
- 错误类型统计

### 日志记录
- 预签名URL生成
- 文件上传验证
- 错误详情记录
- 性能指标追踪

这个架构设计确保了文件上传的安全性、可靠性和高性能，同时支持多种使用场景和存储提供商。 