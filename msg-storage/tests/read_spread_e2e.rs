//! 读扩散端到端验证（真实 PostgreSQL + Redis）
//!
//! 默认 `#[ignore]`，因为它依赖本地容器。执行方式：
//!
//! ```bash
//! docker exec -i rustim_postgres psql -U kelisi -d rustim < docs/MESSAGES_STORAGE_DDL.sql
//! cargo test -p msg-storage --test read_spread_e2e -- --ignored --nocapture
//! ```
//!
//! 覆盖：会话序号分配、幂等重放、正文内联与回表、会话水位、成员登记、未读与已读游标。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use config::{Config as Cfg, File, FileFormat};
use contract::proto::message::{Msg, MsgType};
use msg_storage::read_spread::group_conversation_id;
use msg_storage::{MessageWriter, ReadSpreadRepo};
use runtime::config::AppConfig;

fn load_config() -> AppConfig {
    // 用 include_str 而不是运行时配置文件查找：测试的工作目录是 crate 根，
    // 而配置文件在仓库根，靠当前目录找会失败。
    Cfg::builder()
        .add_source(File::from_str(
            include_str!("../../config/config.yaml"),
            FileFormat::Yaml,
        ))
        .build()
        .expect("读取配置失败")
        .try_deserialize()
        .expect("配置结构与 AppConfig 不匹配")
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间异常")
        .as_millis()
}

#[tokio::test]
#[ignore = "需要本地 PostgreSQL 与 Redis"]
async fn write_fetch_and_idempotency() {
    let config = load_config();
    let cache = cache::cache(&config).await;
    let repo = Arc::new(
        ReadSpreadRepo::from_config(&config)
            .await
            .expect("建表检查失败，请先执行 docs/MESSAGES_STORAGE_DDL.sql"),
    );
    let writer = MessageWriter::new(
        repo.clone(),
        cache,
        config.message_storage.inline_content_max_bytes,
    );

    // 每次运行用独立会话，避免历史数据干扰断言
    let suffix = unique_suffix();
    let group_id = format!("g_e2e_{}", suffix);
    let conversation_id = group_conversation_id(&group_id);

    let members: Vec<String> = ["u1", "u2", "u3"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let base = Msg {
        server_id: format!("srv1_{}", suffix),
        send_id: "u1".into(),
        receiver_id: group_id.clone(),
        group_id: group_id.clone(),
        msg_type: MsgType::GroupMsg as i32,
        content: b"hello".to_vec(),
        send_time: 1_700_000_000_000,
        ..Default::default()
    };

    // 1. 首条消息：会话从 1 开始编号
    let (conv, seq1, inserted1) = writer.save(base.clone(), &members).await.expect("首次写入失败");
    assert_eq!(conv, conversation_id, "会话ID格式应为 g:{group_id}");
    assert_eq!(seq1, 1, "新会话的首个序号应为 1");
    assert!(inserted1, "首次写入应标记为新写入");

    // 2. 幂等重放：返回同一序号，且不再写入
    let (_, seq_dup, inserted_dup) = writer
        .save(base.clone(), &members)
        .await
        .expect("重放写入失败");
    assert_eq!(seq_dup, seq1, "重放不应分配新序号");
    assert!(!inserted_dup, "重放应被唯一约束拦下");

    // 3. 第二条消息：正文超过内联阈值，走 msg_content 回表
    let big_content = vec![7u8; 4096];
    let second = Msg {
        server_id: format!("srv2_{}", suffix),
        content: big_content.clone(),
        send_time: 1_700_000_001_000,
        ..base.clone()
    };
    let (_, seq2, inserted2) = writer.save(second, &members).await.expect("第二条写入失败");
    assert_eq!(seq2, 2, "同一会话序号应递增");
    assert!(inserted2);

    // 4. 增量拉取：正文应完整（小正文内联、大正文回表）
    let fetched = repo
        .meta
        .fetch_after(&conversation_id, 0, 50)
        .await
        .expect("拉取失败");
    assert_eq!(fetched.len(), 2, "应取回两条消息");
    assert_eq!(fetched[0].seq, 1);
    assert_eq!(fetched[0].content, b"hello", "内联正文应原样返回");
    assert_eq!(fetched[1].seq, 2);
    assert_eq!(fetched[1].content, big_content, "外置正文应回表补齐");

    // 5. 会话水位：消息水位是实际最后一条，号段水位会超前
    let state = repo
        .conversation
        .get(&conversation_id)
        .await
        .expect("查询会话失败")
        .expect("会话应存在");
    assert_eq!(state.last_msg_seq, 2, "last_msg_seq 应为实际最后一条");
    assert!(
        state.last_seq >= state.last_msg_seq,
        "号段水位应不低于消息水位"
    );

    // 6. 成员登记与未读
    let states = repo
        .user_conversation
        .list("u2", 100)
        .await
        .expect("查询用户会话失败");
    assert!(
        states.iter().any(|s| s.conversation_id == conversation_id),
        "成员应被登记到 user_conversation"
    );

    let unread = repo
        .user_conversation
        .unread("u2", &conversation_id)
        .await
        .expect("未读查询失败");
    assert_eq!(unread, 2, "未读应等于消息水位减去已读游标");

    // 7. 标记已读后未读归零
    repo.user_conversation
        .mark_read("u2", &conversation_id, 2)
        .await
        .expect("标记已读失败");
    let unread_after = repo
        .user_conversation
        .unread("u2", &conversation_id)
        .await
        .expect("未读查询失败");
    assert_eq!(unread_after, 0);

    println!(
        "端到端通过: conversation={}, seq=1,2, 内联/外置正文、幂等、未读均符合预期",
        conversation_id
    );
}
