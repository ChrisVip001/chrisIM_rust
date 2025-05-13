// 导入生成的gRPC服务代码
pub mod auth {
    tonic::include_proto!("auth");
}

pub mod user {
    tonic::include_proto!("user");
}

pub mod group {
    tonic::include_proto!("group");
}

pub mod friend {
    tonic::include_proto!("friend");
}

pub mod private_message {
    tonic::include_proto!("private_message");
}

pub mod group_message {
    tonic::include_proto!("group_message");
}

pub mod message_gateway {
    tonic::include_proto!("message_gateway");
} 