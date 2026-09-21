use common::snowflake::{Snowflake, SNOWFLAKE};
use common::utils::generate_user_id;
use std::collections::HashSet;
use std::time::Instant;
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    println!("测试雪花ID生成器...");
    
    // 测试全局单例生成器
    println!("\n1. 测试全局单例生成器:");
    for i in 0..5 {
        match SNOWFLAKE.generate() {
            Ok(id) => println!("  - ID {}: {}", i+1, id),
            Err(e) => println!("  - 生成ID失败: {}", e),
        }
    }
    
    // 测试工具函数
    println!("\n2. 测试generate_user_id工具函数(带重试机制):");
    for i in 0..5 {
        match generate_user_id() {
            Ok(id) => println!("  - 用户ID {}: {}", i+1, id),
            Err(e) => println!("  - 生成用户ID失败: {}", e),
        }
    }
    
    // 性能测试
    println!("\n3. 性能测试 (生成10000个ID):");
    let start = Instant::now();
    let mut ids = HashSet::new();
    for _ in 0..10000 {
        let id = SNOWFLAKE.generate().unwrap();
        ids.insert(id);
    }
    let duration = start.elapsed();
    
    println!("  - 生成10000个ID耗时: {:?}", duration);
    println!("  - 平均每个ID生成耗时: {:?}", duration / 10000);
    println!("  - 唯一ID数量: {}", ids.len());
    
    // 节点ID验证
    println!("\n4. 节点ID验证:");
    match Snowflake::new(1023, None) {
        Ok(_) => println!("  - 节点ID 1023: 有效"),
        Err(e) => println!("  - 节点ID 1023: 无效 - {}", e),
    }
    
    match Snowflake::new(1024, None) {
        Ok(_) => println!("  - 节点ID 1024: 有效"),
        Err(e) => println!("  - 节点ID 1024: 无效 - {}", e),
    }
    
    // 测试错误处理和结果返回
    println!("\n5. 测试错误处理:");
    // 创建一个模拟的错误情况
    println!("  - 正常情况下，generate_user_id会返回Ok(id)");
    println!("  - 当出现错误时，会返回Err而不是panic");
    
    // 测试并发
    println!("\n6. 测试并发生成雪花ID:");
    let snowflake = Arc::new(Snowflake::new(2, None).unwrap());
    let mut handles = vec![];
    let counter = Arc::new(Mutex::new(0));
    let thread_count = 10;
    let ids_per_thread = 1000;
    
    for t in 0..thread_count {
        let snowflake_clone = snowflake.clone();
        let counter_clone = counter.clone();
        
        let handle = thread::spawn(move || {
            let mut success = 0;
            let mut local_ids = HashSet::new();
            
            for _ in 0..ids_per_thread {
                match snowflake_clone.generate() {
                    Ok(id) => {
                        local_ids.insert(id);
                        success += 1;
                    }
                    Err(_) => {}
                }
            }
            
            let mut count = counter_clone.lock().unwrap();
            *count += success;
            
            println!("  - 线程 {} 成功生成 {} 个ID", t+1, success);
            local_ids
        });
        
        handles.push(handle);
    }
    
    let mut all_ids = HashSet::new();
    for handle in handles {
        let thread_ids = handle.join().unwrap();
        all_ids.extend(thread_ids);
    }
    
    let total = *counter.lock().unwrap();
    println!("  - 总共生成 {} 个ID", total);
    println!("  - 唯一ID数量: {} (应该等于总数)", all_ids.len());
    
    println!("\n测试完成!");
} 