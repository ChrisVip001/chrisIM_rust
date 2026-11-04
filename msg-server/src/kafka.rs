//! Kafka 生产者与主题管理的公共封装
//!
//! 生产者一度是内联在 `productor` 里的，消费者需要死信投递时又要建一个，
//! 于是把这部分收敛到这里：两边共用同一套可靠投递参数，避免参数漂移。

use std::time::Duration;

use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use tracing::{error, info};

use runtime::config::AppConfig;
use contract::error::Error;

/// 构建生产者
///
/// 参数围绕「不丢消息」选择：
/// - `enable.idempotence=true` 配合 `acks=all` 与无限重试，保证重试不会产生重复消息；
/// - 幂等模式下 `max.in.flight.requests.per.connection` 必须 ≤ 5，否则顺序保证失效；
/// - `snappy` 压缩在 IM 这种小消息为主的场景下带宽收益明显。
pub(crate) fn build_producer(config: &AppConfig) -> Result<FutureProducer, Error> {
    ClientConfig::new()
        .set("bootstrap.servers", config.kafka.hosts.join(","))
        .set("message.timeout.ms", "30000")
        .set("queue.buffering.max.messages", "100000")
        .set("queue.buffering.max.kbytes", "1048576")
        .set("batch.num.messages", "1000")
        .set("enable.idempotence", "true")
        .set("retries", "2147483647")
        .set("max.in.flight.requests.per.connection", "5")
        .set("acks", "all")
        .set("compression.type", "snappy")
        .create()
        .map_err(|e| Error::Internal(format!("创建Kafka生产者失败: {}", e)))
}

/// 确保主题存在
///
/// 采用「先创建后判断」的策略：已存在时 AdminClient 会返回
/// `AdminOpCreation` 错误，这里视作成功。
pub(crate) async fn ensure_topic(
    topic: &str,
    brokers: &str,
    timeout: u16,
) -> Result<(), KafkaError> {
    let admin_client: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("socket.timeout.ms", timeout.to_string())
        .create()?;

    // 分区数与副本数保持与历史配置一致，避免既有部署发生重平衡
    let new_topics = [NewTopic {
        name: topic,
        num_partitions: 1,
        replication: TopicReplication::Fixed(1),
        config: vec![],
    }];

    let options = AdminOptions::new();
    match admin_client.create_topics(&new_topics, &options).await {
        Ok(_) => {
            info!("Kafka主题 '{}' 不存在，已成功创建", topic);
            Ok(())
        }
        Err(KafkaError::AdminOpCreation(_)) => {
            info!("Kafka主题 '{}' 已存在，无需创建", topic);
            Ok(())
        }
        Err(err) => {
            error!("创建Kafka主题 '{}' 失败: {:?}", topic, err);
            Err(err)
        }
    }
}

/// 向指定主题投递一条消息
///
/// 返回错误时表示**没有**投递成功，调用方可以据此决定是否推进消费位点。
pub(crate) async fn send_to_topic(
    producer: &FutureProducer,
    topic: &str,
    payload: &str,
    key: &str,
) -> Result<(), KafkaError> {
    let record: FutureRecord<'_, str, str> = FutureRecord::to(topic).payload(payload).key(key);
    producer
        .send(record, Duration::from_secs(10))
        .await
        .map(|_| ())
        .map_err(|(e, _)| e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdkafka::consumer::{Consumer, StreamConsumer};
    use rdkafka::Message;

    /// 死信投递的端到端验证
    ///
    /// 消费者把处理失败的消息写进死信主题，运维才能重放；
    /// 这里确认负载能被原样投递并读回。需要真实 Kafka，通过
    /// `IM_TEST_KAFKA_BROKERS` 指定，未设置时跳过。
    #[tokio::test]
    async fn dead_letter_payload_can_be_delivered_and_read_back() {
        let Some(brokers) = std::env::var("IM_TEST_KAFKA_BROKERS")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "跳过 dead_letter_payload_can_be_delivered_and_read_back：未设置 IM_TEST_KAFKA_BROKERS"
            );
            return;
        };

        // 每次运行使用独立主题，避免读到历史消息导致断言失败
        let topic = format!("im-test-dlq-{}", std::process::id());
        ensure_topic(&topic, &brokers, 5000)
            .await
            .expect("创建测试主题失败");

        let producer = ClientConfig::new()
            .set("bootstrap.servers", &brokers)
            .create::<FutureProducer>()
            .expect("创建测试生产者失败");

        let payload = r#"{"server_id":"s-dlq-1","reason":"handler failed"}"#;
        send_to_topic(&producer, &topic, payload, "test-key")
            .await
            .expect("死信投递失败");

        let consumer: StreamConsumer = ClientConfig::new()
            .set("group.id", format!("im-test-dlq-{}", std::process::id()))
            .set("bootstrap.servers", &brokers)
            .set("auto.offset.reset", "earliest")
            .create()
            .expect("创建测试消费者失败");

        consumer
            .subscribe(&[topic.as_str()])
            .expect("订阅测试主题失败");

        let message = tokio::time::timeout(Duration::from_secs(30), consumer.recv())
            .await
            .expect("等待死信超时")
            .expect("读取死信失败");

        let received = message
            .payload_view::<str>()
            .expect("死信负载为空")
            .expect("死信负载不是合法 UTF-8");

        assert_eq!(received, payload, "死信必须原样保留负载，否则无法重放");
    }
}
