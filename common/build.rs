fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 告诉Cargo如果proto文件发生变化，就重新运行此构建脚本
    println!("cargo:rerun-if-changed=proto/");

    // 打印当前目录
    println!("Current directory: {:?}", std::env::current_dir()?);

    // 创建输出目录，以防它不存在
    let out_dir = "src/proto";
    std::fs::create_dir_all(out_dir)?;
    println!("Created output directory: {}", out_dir);

    // 定义所有proto文件
    let proto_files = [
        "auth.proto",
        "user.proto",
        "friend.proto",
        "group.proto",
        "private_message.proto",
        "group_message.proto",
        "message_gateway.proto",
    ];

    // 编译所有proto文件并生成文件描述符集
    for proto_file in &proto_files {
        let name = proto_file.strip_suffix(".proto").unwrap_or(proto_file);
        let descriptor_name = format!("{}_descriptor", name);

        tonic_build::configure()
            .build_client(true) // 生成客户端代码
            .build_server(true) // 生成服务器代码
            .file_descriptor_set_path(format!(
                "{}/{}.bin",
                std::env::var("OUT_DIR")?,
                descriptor_name
            ))
            .compile(
                // 指定要编译的proto文件
                &[format!("proto/{}", proto_file)],
                // 指定proto文件的搜索路径，用于解析import语句
                &["proto"],
            )?;
    }

    // 自动生成服务客户端封装代码
    generate_service_clients()?;

    Ok(())
}

// 自动生成服务客户端封装代码的函数
fn generate_service_clients() -> Result<(), Box<dyn std::error::Error>> {
    // 创建输出目录
    let out_dir = "src/grpc_client/generated";
    std::fs::create_dir_all(out_dir)?;
    
    // 定义所有需要生成客户端的服务
    let services = [
        ("user", "User"),
        ("friend", "Friend"),
        ("group", "Group"),
        ("auth", "Auth"),
        // 可以继续添加其他服务...
    ];
    
    // 生成mod.rs文件导出所有生成的客户端
    let mut mod_content = String::new();
    mod_content.push_str("// 自动生成的服务客户端导出\n\n");
    
    for (service_name, service_upper) in &services {
        // 生成每个服务的客户端代码
        let client_code = generate_client_code(service_name, service_upper)?;
        let file_name = format!("{}_client_gen.rs", service_name);
        std::fs::write(format!("{}/{}", out_dir, file_name), client_code)?;
        
        // 添加到mod.rs
        mod_content.push_str(&format!("pub mod {}_client_gen;\n", service_name));
        mod_content.push_str(&format!("pub use {}_client_gen::{}ServiceGrpcClientGen;\n", service_name, service_upper));
    }
    
    // 写入mod.rs
    std::fs::write(format!("{}/mod.rs", out_dir), mod_content)?;
    
    // 更新主mod.rs添加generated模块
    let main_mod_path = "src/grpc_client/mod.rs";
    let mut main_mod_content = match std::fs::read_to_string(main_mod_path) {
        Ok(content) => content,
        Err(_) => "".to_string(),
    };
    
    // 检查是否已经添加generated模块
    if !main_mod_content.contains("pub mod generated") {
        main_mod_content.push_str("\n// 自动生成的客户端模块\npub mod generated;\n");
        main_mod_content.push_str("pub use generated::*;\n");
        std::fs::write(main_mod_path, main_mod_content)?;
    }
    
    Ok(())
}

// 为指定服务生成客户端代码
fn generate_client_code(service_name: &str, service_upper: &str) -> Result<String, Box<dyn std::error::Error>> {
    let service_proto_path = format!("crate::proto::{}", service_name);
    let service_client = format!("{}ServiceClient", service_upper);
    let service_client_path = format!("{}::{}::{}",
        service_proto_path, 
        format!("{}_service_client", service_name), 
        service_client
    );
    
    // 生成客户端代码
    let code = format!(r#"
use anyhow::Result;
use tonic::Request;
use crate::grpc_client::GrpcServiceClient;
use {}::*;

/// 自动生成的{}服务gRPC客户端
#[derive(Clone)]
pub struct {}ServiceGrpcClientGen {{
    service_client: GrpcServiceClient,
}}

impl {}ServiceGrpcClientGen {{
    /// 创建新的{}服务客户端
    pub fn new(service_client: GrpcServiceClient) -> Self {{
        Self {{ service_client }}
    }}

    /// 从环境变量创建客户端
    pub fn from_env() -> Self {{
        let service_client = GrpcServiceClient::from_env("{}-service");
        Self::new(service_client)
    }}

    /// 获取底层服务客户端
    async fn get_client(&self) -> Result<{}> {{
        let channel = self.service_client.get_channel().await?;
        Ok({}::new(channel))
    }}
    
    // 这里可以自动生成各个服务方法的封装
    // 由于需要知道每个服务的具体方法，可能需要解析proto文件
    // 或者提供一个通用方法
    
    /// 执行通用的服务调用
    pub async fn call<T, R>(&self, method_name: &str, request: T) -> Result<R> 
    where
        T: prost::Message,
        R: prost::Message + Default,
    {{
        let mut client = self.get_client().await?;
        // 这里需要通过反射或其他方式调用指定方法
        // 实现复杂度高，可能需要使用unsafe或宏
        unimplemented!("通用调用方法需要更复杂的实现")
    }}
}}
"#, service_proto_path, service_upper, service_upper, service_upper, service_upper, service_name, service_client, service_client_path);

    Ok(code)
}
