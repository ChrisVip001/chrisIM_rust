fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 告诉Cargo如果proto文件发生变化，就重新运行此构建脚本
    println!("cargo:rerun-if-changed=proto/");
    
    // 打印当前目录
    println!("Current directory: {:?}", std::env::current_dir()?);
    
    // 创建输出目录，以防它不存在
    let out_dir = "src/proto";
    std::fs::create_dir_all(out_dir)?;
    println!("Created output directory: {}", out_dir);
    
    // 编译所有proto文件
    // 使用tonic_build的configure方法来自定义生成的代码
    // 在tonic-build 0.13.0版本中，应该使用compile_protos方法
    tonic_build::configure()
        .build_client(true)  // 生成客户端代码
        .build_server(true)  // 生成服务器代码
        .compile(
            // 指定要编译的所有proto文件
            &[
                "proto/auth.proto",
                "proto/user.proto",
                "proto/friend.proto",
                "proto/group.proto",
                "proto/private_message.proto",
                "proto/group_message.proto",
                "proto/message_gateway.proto",
            ],
            // 指定proto文件的搜索路径，用于解析import语句
            &["proto"],
        )?;
    
    println!("Successfully compiled all proto files");
    
    Ok(())
} 