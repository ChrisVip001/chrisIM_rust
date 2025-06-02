/// Protobuf编译构建脚本
/// 
/// 这个构建脚本负责将.proto文件编译成Rust代码，生成gRPC客户端和服务器代码。
/// 使用tonic_build库来处理protobuf编译，支持Google标准类型和proto3可选字段。
/// 
/// 编译的proto文件包括：
/// - user.proto: 用户服务相关的消息和接口定义
/// - friend.proto: 好友服务相关的消息和接口定义  
/// - group.proto: 群组服务相关的消息和接口定义
/// - messages.proto: 消息系统的核心数据结构和接口定义
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Cargo如果proto文件发生变化，就重新运行此构建脚本
    println!("cargo:rerun-if-changed=proto/");

    // 打印当前目录
    println!("Current directory: {:?}", std::env::current_dir()?);

    // 生成的Rust代码将放在src/proto目录下
    let out_dir = "src/proto";
    std::fs::create_dir_all(out_dir)?;
    println!("Created output directory: {}", out_dir);

    // 定义所有需要编译的proto文件
    // 这些文件定义了整个即时通讯系统的数据结构和服务接口
    let proto_files = [
        "user.proto",      // 用户管理相关接口
        "friend.proto",    // 好友关系管理接口
        "group.proto",     // 群组管理接口
        "messages.proto",  // 消息系统核心接口
    ];

    // 编译所有proto文件并生成文件描述符集
    // 为每个proto文件单独配置编译选项
    for proto_file in &proto_files {
        // 从文件名中提取基础名称（去掉.proto后缀）
        let name = proto_file.strip_suffix(".proto").unwrap_or(proto_file);
        // 生成对应的描述符文件名
        let descriptor_name = format!("{}_descriptor", name);

        // 配置tonic构建器
        let mut config = tonic_build::configure()
            .build_client(true) // 生成客户端代码，用于调用其他服务
            .build_server(true) // 生成服务器代码，用于实现gRPC服务
            .file_descriptor_set_path(format!(
                "{}/{}.bin",
                std::env::var("OUT_DIR")?,  // Cargo提供的输出目录
                descriptor_name
            ))
            .compile_well_known_types(true) // 启用Google标准类型支持（如Timestamp、Duration等）
            .extern_path(".google.protobuf", "::prost_types") // 使用prost_types作为Google protobuf类型的外部路径
            .protoc_arg("--experimental_allow_proto3_optional"); // 支持proto3的optional字段特性

        // 只为特定的消息类型添加 serde 支持，避免 Timestamp 问题
        if *proto_file == "messages.proto" {
            config = config
                .type_attribute(".messages.Msg", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.GroupMemSeq", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.MsgRead", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.MsgType", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.ContentType", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.PlatformType", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.Candidate", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.MsgResponse", "#[derive(serde::Serialize, serde::Deserialize)]")
                .type_attribute(".messages.SendMsgResponse", "#[derive(serde::Serialize, serde::Deserialize)]")
                .field_attribute(".messages.Msg", "#[serde(default)]")
                .field_attribute(".messages.GroupMemSeq", "#[serde(default)]")
                .field_attribute(".messages.MsgRead", "#[serde(default)]");
        }

        config.compile(
            // 指定要编译的proto文件路径
            &[format!("proto/{}", proto_file)],
            // 指定proto文件的搜索路径，用于解析import语句
            // 当proto文件中有import其他proto文件时，编译器会在这些路径中查找
            &["proto"],
        )?;
    }

    Ok(())
}
