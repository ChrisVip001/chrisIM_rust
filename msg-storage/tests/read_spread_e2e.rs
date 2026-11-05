//! 读扩散端到端验证（真实 PostgreSQL + Redis）
//!
//! 默认 `#[ignore]`，因为它依赖本地容器。执行方式：
//!
//! ```bash
//! docker exec -i rustim_postgres psql -U kelisi -d rustim < docs/sql/init.sql
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
            .expect("建表检查失败，请先执行 docs/sql/init.sql"),
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
    //
    // 消息水位不再随单条写入更新（那是同一行被高频改写的热点），
    // 改由调用方按批汇总回写一次；这里显式验证「回写前不前进、回写后前进」。
    let before = repo
        .conversation
        .get(&conversation_id)
        .await
        .expect("查询会话失败")
        .expect("会话应存在");
    assert_eq!(
        before.last_msg_seq, 0,
        "落库不应再顺带更新会话水位，水位由批次回写负责"
    );

    writer
        .touch_watermarks(&[(conversation_id.clone(), 2, 1_700_000_001_000)])
        .await
        .expect("回写会话水位失败");

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

    // 回写只增不减：重复批次不能把水位拉回去
    writer
        .touch_watermarks(&[(conversation_id.clone(), 1, 1_700_000_000_000)])
        .await
        .expect("重复回写失败");
    let state = repo
        .conversation
        .get(&conversation_id)
        .await
        .expect("查询会话失败")
        .expect("会话应存在");
    assert_eq!(state.last_msg_seq, 2, "水位回写必须只增不减");

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

/// 批量写入必须与逐条写入等价
///
/// 覆盖：返回顺序与入参一致、序号按入参顺序连续、重放不消耗号段、
/// 内联与外置正文都能取回、参与者仍然登记。
/// 这是满载改造的正确性底线——批量只为省语句数，不改任何可观测语义。
#[tokio::test]
#[ignore = "需要本地 PostgreSQL 与 Redis"]
async fn batch_write_matches_single_write() {
    let config = load_config();
    let cache = cache::cache(&config).await;
    let repo = Arc::new(
        ReadSpreadRepo::from_config(&config)
            .await
            .expect("建表检查失败，请先执行 docs/sql/init.sql"),
    );
    let writer = MessageWriter::new(
        repo.clone(),
        cache,
        config.message_storage.inline_content_max_bytes,
    );

    let suffix = unique_suffix();
    let group_id = format!("g_batch_{}", suffix);
    let conversation_id = group_conversation_id(&group_id);
    let members: Vec<String> = ["u1", "u2"].iter().map(|s| s.to_string()).collect();

    let base = Msg {
        send_id: "u1".into(),
        receiver_id: group_id.clone(),
        group_id: group_id.clone(),
        msg_type: MsgType::GroupMsg as i32,
        send_time: 1_700_000_000_000,
        ..Default::default()
    };
    let build = |i: usize, content: Vec<u8>| Msg {
        server_id: format!("batch_{}_{}", suffix, i),
        content,
        send_time: 1_700_000_000_000 + i as i64,
        ..base.clone()
    };

    let batch = vec![
        build(0, b"one".to_vec()),
        build(1, vec![9u8; 4096]), // 超过内联阈值，走 msg_content 回表
        build(2, b"three".to_vec()),
    ];

    let saved = writer
        .save_batch(batch.clone(), &members)
        .await
        .expect("批量写入失败");
    assert_eq!(saved.len(), 3);
    for (i, (server_id, seq, inserted)) in saved.iter().enumerate() {
        assert_eq!(
            server_id,
            &format!("batch_{}_{}", suffix, i),
            "返回顺序必须与入参一致"
        );
        assert_eq!(*seq, (i + 1) as i64, "序号应按入参顺序连续分配");
        assert!(*inserted, "首次批量写入应全部标记为新写入");
    }

    // 重放同一批：不分配新序号、不再写入
    let replay = writer
        .save_batch(batch.clone(), &members)
        .await
        .expect("重放失败");
    for (i, (_, seq, inserted)) in replay.iter().enumerate() {
        assert_eq!(*seq, (i + 1) as i64, "重放不应分配新序号");
        assert!(!inserted, "重放应被唯一约束拦下");
    }

    // 正文完整：内联原样返回，外置回表补齐
    let fetched = repo
        .meta
        .fetch_after(&conversation_id, 0, 50)
        .await
        .expect("拉取失败");
    assert_eq!(fetched.len(), 3, "三条消息都应落库");
    assert_eq!(fetched[0].content, b"one");
    assert_eq!(fetched[1].content, vec![9u8; 4096], "外置正文应回表补齐");
    assert_eq!(fetched[2].content, b"three");

    // 参与者登记仍然生效（批量路径不能漏登记）
    let states = repo
        .user_conversation
        .list("u2", 100)
        .await
        .expect("查询用户会话失败");
    assert!(
        states.iter().any(|s| s.conversation_id == conversation_id),
        "成员应被登记到 user_conversation"
    );

    println!(
        "批量写入通过: conversation={}, seq=1..3 连续、重放不消耗号段、内联/外置正文一致",
        conversation_id
    );
}

/// 跨会话批量写入：整批一条语句，各会话序号互不影响
///
/// 这是满载主路径（`save_batch_multi`）的正确性底线：
/// 会话 A、B 的消息混在一批里，各自的序号必须从 1 开始独立递增，
/// 重放仍不消耗号段，成员按会话分别登记。
#[tokio::test]
#[ignore = "需要本地 PostgreSQL 与 Redis"]
async fn batch_write_spans_conversations() {
    let config = load_config();
    let cache = cache::cache(&config).await;
    let repo = Arc::new(
        ReadSpreadRepo::from_config(&config)
            .await
            .expect("建表检查失败，请先执行 docs/sql/init.sql"),
    );
    let writer = MessageWriter::new(
        repo.clone(),
        cache,
        config.message_storage.inline_content_max_bytes,
    );

    let suffix = unique_suffix();
    let group_a = format!("g_multi_a_{}", suffix);
    let group_b = format!("g_multi_b_{}", suffix);
    let conv_a = group_conversation_id(&group_a);
    let conv_b = group_conversation_id(&group_b);
    let members_a: Vec<String> = ["u1", "u2"].iter().map(|s| s.to_string()).collect();
    let members_b: Vec<String> = ["u3", "u4"].iter().map(|s| s.to_string()).collect();

    let build = |group: &str, i: usize, send_id: &str| Msg {
        server_id: format!("multi_{}_{}_{}", suffix, group, i),
        send_id: send_id.into(),
        receiver_id: group.into(),
        group_id: group.into(),
        msg_type: MsgType::GroupMsg as i32,
        content: format!("m{}", i).into_bytes(),
        send_time: 1_700_000_000_000 + i as i64,
        ..Default::default()
    };

    // 交错排列：A1 B1 A2 B2，验证批内顺序与会话内顺序互不干扰
    let items = vec![
        (build(&group_a, 1, "u1"), members_a.clone()),
        (build(&group_b, 1, "u3"), members_b.clone()),
        (build(&group_a, 2, "u1"), members_a.clone()),
        (build(&group_b, 2, "u3"), members_b.clone()),
    ];

    let saved = writer
        .save_batch_multi(items.clone())
        .await
        .expect("跨会话批量写入失败");
    assert_eq!(saved.len(), 4);

    let seqs: Vec<(String, i64, bool)> = saved.clone();
    assert_eq!(seqs[0].0, conv_a, "第 1 条应落在会话 A");
    assert_eq!(seqs[0].1, 1);
    assert!(seqs[0].2);
    assert_eq!(seqs[1].0, conv_b, "第 2 条应落在会话 B");
    assert_eq!(seqs[1].1, 1, "会话 B 的序号独立从 1 开始");
    assert!(seqs[1].2);
    assert_eq!(seqs[2].0, conv_a);
    assert_eq!(seqs[2].1, 2, "会话 A 的第二条应为 2");
    assert_eq!(seqs[3].1, 2, "会话 B 的第二条应为 2");

    // 重放整批：不分配新序号
    let replay = writer
        .save_batch_multi(items)
        .await
        .expect("跨会话重放失败");
    for (i, (_, seq, inserted)) in replay.iter().enumerate() {
        assert_eq!(*seq, ((i / 2) + 1) as i64, "重放不应分配新序号");
        assert!(!inserted, "重放应被唯一约束拦下");
    }

    // 两个会话的正文各自独立可取
    let a = repo.meta.fetch_after(&conv_a, 0, 10).await.expect("拉取 A 失败");
    let b = repo.meta.fetch_after(&conv_b, 0, 10).await.expect("拉取 B 失败");
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 2);
    assert_eq!(a[0].content, b"m1");
    assert_eq!(b[0].content, b"m1");

    println!(
        "跨会话批量写入通过: A={}, B={}，序号独立、重放不消耗号段",
        conv_a, conv_b
    );
}

#[tokio::test]
#[ignore = "需要本地 PostgreSQL 与 Redis"]
async fn batch_with_dirty_entry_does_not_block_others() {
    // 同批混入 server_id 为空的消息：不能因为唯一约束冲突让整批回滚。
    // 这条用例守的是 ON CONFLICT 不带 target 的兜底行为（主键 + server_id 唯一索引全覆盖）。
    let config = load_config();
    let cache = cache::cache(&config).await;
    let repo = Arc::new(
        ReadSpreadRepo::from_config(&config)
            .await
            .expect("建表检查失败，请先执行 docs/sql/init.sql"),
    );
    let writer = MessageWriter::new(
        repo.clone(),
        cache,
        config.message_storage.inline_content_max_bytes,
    );

    let suffix = unique_suffix();
    let group_id = format!("g_dirty_{}", suffix);
    let conversation_id = group_conversation_id(&group_id);
    let members: Vec<String> = ["u1", "u2"].iter().map(|s| s.to_string()).collect();

    let good = Msg {
        server_id: format!("good_{}", suffix),
        send_id: "u1".into(),
        receiver_id: group_id.clone(),
        group_id: group_id.clone(),
        msg_type: MsgType::GroupMsg as i32,
        content: b"ok".to_vec(),
        send_time: 1_700_000_000_000,
        ..Default::default()
    };
    // 故意构造脏数据：server_id 为空（无法作为幂等键，且空串之间会互相冲突）
    let dirty = Msg {
        server_id: String::new(),
        send_time: 1_700_000_001_000,
        ..good.clone()
    };

    let saved = writer
        .save_batch_multi(vec![
            (good.clone(), members.clone()),
            (dirty, members.clone()),
        ])
        .await
        .expect("含脏消息的整批不应失败（否则一条坏数据会连坐同批所有会话）");

    // 返回的是 (conversation_id, seq, inserted)
    assert_eq!(saved.len(), 2, "整批两条都应写入，而不是因脏数据回滚: {saved:?}");
    assert!(
        saved.iter().all(|(_, _, inserted)| *inserted),
        "两条都应标记为新写入: {saved:?}"
    );

    let fetched = repo
        .meta
        .fetch_after(&conversation_id, 0, 50)
        .await
        .expect("拉取失败");
    assert!(
        fetched.iter().any(|m| m.server_id == good.server_id),
        "正常消息应可被读出"
    );

    println!("脏消息不连坐通过: {} 条正常消息已落库", saved.len());
}

/// 会话列表索引与批量查询（真实 Redis + PostgreSQL）
///
/// 覆盖：`ZADD GT` 只增不减、活跃时间倒序、`get_many` 与 `list_by_ids` 的批量语义、
/// 以及 `list` 按最后消息时间倒序。
#[tokio::test]
#[ignore = "需要本地 PostgreSQL 与 Redis"]
async fn conversation_index_and_batch_lookup() {
    use cache::ConversationIndexCache;

    let config = load_config();
    let cache = cache::cache(&config).await;
    let repo = Arc::new(
        ReadSpreadRepo::from_config(&config)
            .await
            .expect("建表检查失败，请先执行 docs/sql/init.sql"),
    );
    let writer = MessageWriter::new(
        repo.clone(),
        cache.clone(),
        config.message_storage.inline_content_max_bytes,
    );

    let suffix = unique_suffix();
    let user_id = format!("u_e2e_{}", suffix);
    let older_group = format!("g_older_{}", suffix);
    let newer_group = format!("g_newer_{}", suffix);
    let older_conv = group_conversation_id(&older_group);
    let newer_conv = group_conversation_id(&newer_group);
    let members = vec![user_id.clone()];

    // 两个会话各写一条消息，活跃时间一老一新
    for (group_id, server_id, send_time) in [
        (older_group.clone(), format!("old_{}", suffix), 1_700_000_000_000),
        (newer_group.clone(), format!("new_{}", suffix), 1_700_000_100_000),
    ] {
        let msg = Msg {
            server_id,
            send_id: user_id.clone(),
            receiver_id: group_id.clone(),
            group_id,
            msg_type: MsgType::GroupMsg as i32,
            content: b"hi".to_vec(),
            send_time,
            ..Default::default()
        };
        writer
            .save(msg, &members)
            .await
            .expect("写入消息失败");
    }

    // 会话水位回写后，list 才能按 last_msg_time 排序
    writer
        .touch_watermarks(&[
            (older_conv.clone(), 1, 1_700_000_000_000),
            (newer_conv.clone(), 1, 1_700_000_100_000),
        ])
        .await
        .expect("回写会话水位失败");

    // 1. 批量取会话元信息：两条都应有，且最近消息时间正确
    let states = repo
        .conversation
        .get_many(&[older_conv.clone(), newer_conv.clone()])
        .await
        .expect("批量查询会话失败");
    assert_eq!(states.len(), 2, "批量查询应返回两个会话: {states:?}");
    let newer = states
        .iter()
        .find(|c| c.conversation_id == newer_conv)
        .expect("应包含新会话");
    assert_eq!(newer.last_msg_time, 1_700_000_100_000);

    // 不存在的会话不出现在结果里
    let missing = repo
        .conversation
        .get_many(&[format!("g:not_exists_{}", suffix)])
        .await
        .expect("批量查询不存在的会话不应报错");
    assert!(missing.is_empty());

    // 2. list 按最后消息时间倒序：新会话在前
    let listed = repo
        .user_conversation
        .list(&user_id, 10)
        .await
        .expect("查询用户会话失败");
    let listed_ids: Vec<&str> = listed.iter().map(|s| s.conversation_id.as_str()).collect();
    assert_eq!(
        listed_ids,
        vec![newer_conv.as_str(), older_conv.as_str()],
        "会话列表应按最后消息时间倒序"
    );

    // 3. list_by_ids 只返回请求的会话
    let only_older = repo
        .user_conversation
        .list_by_ids(&user_id, std::slice::from_ref(&older_conv))
        .await
        .expect("按 ID 查询用户会话失败");
    assert_eq!(only_older.len(), 1);
    assert_eq!(only_older[0].conversation_id, older_conv);

    // 4. 索引：按活跃时间倒序
    let index_user = format!("u_index_{}", suffix);
    cache
        .touch_conversation_index(&[
            (index_user.clone(), older_conv.clone(), 1_700_000_000_000),
            (index_user.clone(), newer_conv.clone(), 1_700_000_100_000),
        ])
        .await
        .expect("写入会话索引失败");

    let recent = cache
        .recent_conversations(&index_user, 10)
        .await
        .expect("读取会话索引失败");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0], (newer_conv.clone(), 1_700_000_100_000), "最近活跃的应排最前");

    // 5. ZADD GT：用更早的时间回写不应把会话往后挪
    cache
        .touch_conversation_index(&[(
            index_user.clone(),
            newer_conv.clone(),
            1_600_000_000_000,
        )])
        .await
        .expect("回写更早时间失败");
    let recent = cache
        .recent_conversations(&index_user, 10)
        .await
        .expect("读取会话索引失败");
    assert_eq!(
        recent[0],
        (newer_conv, 1_700_000_100_000),
        "分数只增不减：更早的时间不能覆盖已有分数"
    );

    println!("会话索引与批量查询通过");
}
