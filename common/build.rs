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
        "user.proto",
        "friend.proto",
        "group.proto",
        "messages.proto",
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

    Ok(())
}
