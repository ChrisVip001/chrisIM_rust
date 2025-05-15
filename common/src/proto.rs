// 导入生成的gRPC服务代码
pub mod auth {
    tonic::include_proto!("auth");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("auth_descriptor");
}

pub mod user {
    tonic::include_proto!("user");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("user_descriptor");
}

pub mod group {
    tonic::include_proto!("group");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("group_descriptor");
}

pub mod friend {
    tonic::include_proto!("friend");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("friend_descriptor");
}

pub mod private_message {
    tonic::include_proto!("private_message");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("private_message_descriptor");
}

pub mod group_message {
    tonic::include_proto!("group_message");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("group_message_descriptor");
}

pub mod message_gateway {
    tonic::include_proto!("message_gateway");

    // 生成用于反射的文件描述符集
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("message_gateway_descriptor");
}
