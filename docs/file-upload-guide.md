# 文件上传指南

本文档介绍如何在RustIM中实现安全的文件上传功能。

## 架构设计

为了减轻服务器压力，文件上传采用前端直传方案，通过预签名URL实现安全上传：

```
┌─────────────┐                  ┌─────────────┐                  ┌─────────────┐
│   前端客户端  │                  │   API网关    │                  │  对象存储服务 │
└──────┬──────┘                  └──────┬──────┘                  └──────┬──────┘
       │                                │                                │
       │ 1. 请求预签名URL               │                                │
       │────────────────────────────────>                                │
       │                                │                                │
       │                                │ 2. 生成预签名URL               │
       │                                │<───────────────────────────────│
       │                                │                                │
       │ 3. 返回预签名URL               │                                │
       │<────────────────────────────────                                │
       │                                │                                │
       │ 4. 直接上传文件到对象存储       │                                │
       │────────────────────────────────────────────────────────────────>│
       │                                │                                │
       │ 5. 上传完成通知                │                                │
       │────────────────────────────────>                                │
       │                                │                                │
       │                                │ 6. 验证文件完整性              │
       │                                │────────────────────────────────>│
       │                                │                                │
       │ 7. 返回验证结果                │                                │
       │<────────────────────────────────                                │
       │                                │                                │
```

## 存储提供商支持

RustIM支持以下对象存储提供商：

1. **S3兼容存储**（默认）：兼容AWS S3 API的存储服务，如MinIO, Ceph等
2. **腾讯云COS**：腾讯云对象存储服务

通过配置文件中的`provider`参数可以选择使用哪种存储服务：

```yaml
# OSS配置
oss:
  # 存储提供商: s3(默认) 或 cos(腾讯云)
  provider: s3
  # S3兼容存储配置
  endpoint: http://127.0.0.1:9000
  access_key: minioadmin
  secret_key: minioadmin
  bucket: rustIM
  avatar_bucket: rustIM-avatar
  region: us-east-1
  # 腾讯云COS特有配置，仅当provider=cos时有效
  cos_app_id: 1250000000 # 替换为您的腾讯云APPID
  cos_domain: https://your-cos-domain.com # 可选的自定义域名
```

## 安全措施

1. **预签名URL**：所有文件上传通过临时预签名URL完成，URL有效期限制（默认15分钟）
2. **认证授权**：获取预签名URL和验证上传需要用户身份验证
3. **文件完整性验证**：通过MD5哈希值验证文件完整性，防止上传过程中的数据损坏
4. **文件大小限制**：前端和后端都实施文件大小限制
5. **文件类型验证**：根据Content-Type验证文件类型
6. **随机文件名**：上传的文件使用UUID随机文件名，防止文件名冲突和信息泄露

## 注册流程中的头像上传

在用户注册过程中，可能需要上传头像，但此时用户尚未登录，没有有效的token。为此，我们提供了专门的头像上传接口，通过API路径白名单机制来允许未认证用户上传头像。

### 注册头像上传流程

1. 用户开始注册流程
2. 前端直接调用 `/api/files/register-avatar` 获取头像上传URL（无需认证）
3. 上传头像到对象存储
4. 调用 `/api/files/validate-register-avatar` 验证头像上传是否成功（无需认证）
5. 完成注册流程，关联头像与用户账号

### 代码示例

```typescript
// register-avatar.ts
import axios from 'axios';
import { message } from 'antd';
import CryptoJS from 'crypto-js';

// 上传注册头像
export const uploadRegisterAvatar = async (
  file: File,
  onProgress?: (percent: number) => void
): Promise<string> => {
  try {
    // 验证文件类型
    if (!['image/jpeg', 'image/png', 'image/webp'].includes(file.type)) {
      throw new Error('不支持的头像格式，仅支持JPG/PNG/WEBP格式');
    }
    
    // 验证文件大小
    if (file.size > 2 * 1024 * 1024) {
      throw new Error('头像文件过大，最大支持2MB');
    }
    
    // 获取预签名URL
    const { data: presignedData } = await axios.post('/api/files/register-avatar', {
      content_type: file.type,
      file_size: file.size,
    });

    if (!presignedData.success) {
      throw new Error(presignedData.message || '获取上传URL失败');
    }

    const { upload_url, key } = presignedData.data;

    // 使用预签名URL上传文件
    await axios.put(upload_url, file, {
      headers: {
        'Content-Type': file.type,
      },
      onUploadProgress: (progressEvent) => {
        if (progressEvent.total && onProgress) {
          const percent = Math.round(
            (progressEvent.loaded * 100) / progressEvent.total
          );
          onProgress(percent);
        }
      },
    });

    // 计算文件MD5用于验证
    const md5 = await calculateMD5(file);

    // 验证上传完成
    const { data: validateData } = await axios.post('/api/files/validate-register-avatar', {
      key,
      size: file.size,
      md5,
    });

    if (!validateData.success) {
      throw new Error(validateData.message || '头像验证失败');
    }

    // 返回头像文件存储键，用于后续注册提交
    return key;
  } catch (error) {
    console.error('头像上传失败:', error);
    message.error('头像上传失败，请重试');
    throw error;
  }
};

// 完整注册流程示例
export const registerWithAvatar = async (
  registerData: {
    username: string;
    password: string;
    phone?: string;
    email?: string;
  },
  avatarFile?: File
) => {
  try {
    // 如果有头像，先上传头像
    let avatarKey: string | undefined;
    if (avatarFile) {
      avatarKey = await uploadRegisterAvatar(avatarFile);
    }
    
    // 完成注册
    const { data } = await axios.post('/api/users/register', {
      ...registerData,
      avatar_key: avatarKey
    });
    
    return data;
  } catch (error) {
    console.error('注册失败:', error);
    throw error;
  }
};

// MD5计算函数
const calculateMD5 = (file: File): Promise<string> => {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      const binary = e.target?.result;
      if (binary) {
        const md5 = CryptoJS.MD5(binary).toString();
        resolve(md5);
      } else {
        reject(new Error('Failed to read file'));
      }
    };
    reader.onerror = (e) => {
      reject(e);
    };
    reader.readAsBinaryString(file);
  });
};
```

## 使用方法

在前端组件中使用注册和头像上传功能：

```tsx
import React, { useState } from 'react';
import { Button, Form, Input, Upload, Progress, message } from 'antd';
import { UploadOutlined, UserOutlined, LockOutlined, MailOutlined } from '@ant-design/icons';
import { uploadRegisterAvatar, registerWithAvatar } from './register-avatar';

const RegisterForm: React.FC = () => {
  const [form] = Form.useForm();
  const [uploading, setUploading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [avatarFile, setAvatarFile] = useState<File | null>(null);
  const [avatarKey, setAvatarKey] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleAvatarUpload = async (file: File) => {
    setAvatarFile(file);
    setUploading(true);
    setProgress(0);
    
    try {
      const key = await uploadRegisterAvatar(file, setProgress);
      setAvatarKey(key);
      message.success('头像上传成功!');
    } catch (error) {
      console.error('头像上传失败:', error);
    } finally {
      setUploading(false);
    }
    
    return false; // 阻止默认上传行为
  };

  const handleSubmit = async (values: any) => {
    setSubmitting(true);
    
    try {
      await registerWithAvatar({
        username: values.username,
        password: values.password,
        email: values.email
      }, avatarFile || undefined);
      
      message.success('注册成功!');
      // 重定向到登录页或其他页面
    } catch (error) {
      console.error('注册失败:', error);
      message.error('注册失败，请重试');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Form form={form} onFinish={handleSubmit} layout="vertical">
      <Form.Item label="头像" extra="可选，最大2MB，支持JPG/PNG/WEBP格式">
        <Upload
          accept="image/jpeg,image/png,image/webp"
          beforeUpload={handleAvatarUpload}
          showUploadList={false}
        >
          <Button 
            icon={<UploadOutlined />} 
            loading={uploading}
            disabled={uploading}
          >
            选择头像
          </Button>
        </Upload>
        
        {uploading && <Progress percent={progress} style={{ marginTop: 8 }} />}
        
        {avatarKey && (
          <div style={{ marginTop: 8 }}>
            头像已上传成功
          </div>
        )}
      </Form.Item>
      
      <Form.Item
        name="username"
        label="用户名"
        rules={[{ required: true, message: '请输入用户名' }]}
      >
        <Input prefix={<UserOutlined />} placeholder="用户名" />
      </Form.Item>
      
      <Form.Item
        name="password"
        label="密码"
        rules={[{ required: true, message: '请输入密码' }]}
      >
        <Input.Password prefix={<LockOutlined />} placeholder="密码" />
      </Form.Item>
      
      <Form.Item
        name="email"
        label="邮箱"
        rules={[
          { required: true, message: '请输入邮箱' },
          { type: 'email', message: '请输入有效的邮箱地址' }
        ]}
      >
        <Input prefix={<MailOutlined />} placeholder="邮箱" />
      </Form.Item>
      
      <Form.Item>
        <Button 
          type="primary" 
          htmlType="submit" 
          loading={submitting}
          disabled={submitting}
          block
        >
          注册
        </Button>
      </Form.Item>
    </Form>
  );
};

export default RegisterForm;
```

## 后端API说明

### 1. 获取注册头像上传URL

```
POST /api/files/register-avatar
```

**请求参数:**
```json
{
  "content_type": "image/jpeg",
  "file_size": 512000
}
```

**响应:**
```json
{
  "success": true,
  "data": {
    "upload_url": "https://oss-endpoint.com/bucket/register_avatars/7f4e6b3a-1c2d-3e4f-5a6b-7c8d9e0f1a2b.jpg?X-Amz-Algorithm=...",
    "key": "register_avatars/7f4e6b3a-1c2d-3e4f-5a6b-7c8d9e0f1a2b.jpg",
    "expires_in": 600
  }
}
```

### 2. 验证注册头像上传

```
POST /api/files/validate-register-avatar
```

**请求参数:**
```json
{
  "key": "register_avatars/7f4e6b3a-1c2d-3e4f-5a6b-7c8d9e0f1a2b.jpg",
  "size": 512000,
  "md5": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"
}
```

**响应:**
```json
{
  "success": true,
  "data": {
    "key": "register_avatars/7f4e6b3a-1c2d-3e4f-5a6b-7c8d9e0f1a2b.jpg",
    "is_valid": true
  }
}
```

### 3. 用户注册（包含头像）

```
POST /api/users/register
```

**请求参数:**
```json
{
  "username": "newuser",
  "password": "secure_password",
  "email": "user@example.com",
  "avatar_key": "register_avatars/7f4e6b3a-1c2d-3e4f-5a6b-7c8d9e0f1a2b.jpg"
}
```

**响应:**
```json
{
  "success": true,
  "data": {
    "user_id": "12345",
    "username": "newuser",
    "avatar_url": "https://oss-endpoint.com/bucket/avatars/12345.jpg"
  }
}
``` 