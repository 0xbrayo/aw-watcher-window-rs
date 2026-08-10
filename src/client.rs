use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use tracing::{debug, info, warn};

#[derive(Debug, Serialize)]
struct BucketInfo<'a> {
    client: &'a str,
    hostname: &'a str,
    #[serde(rename = "type")]
    bucket_type: &'a str,
}

#[derive(Debug, Serialize)]
struct Event<T: Serialize> {
    timestamp: DateTime<Utc>,
    duration: f64,
    data: T,
}

/// HTTP client that can create buckets and heartbeat into multiple bucket ids.
pub struct AwClient {
    http: Client,
    base_url: String,
    hostname: String,
}

impl AwClient {
    pub fn new(host: &str, port: u16, hostname: String) -> Self {
        let base_url = format!("http://{}:{}", host, port);
        Self {
            http: Client::new(),
            base_url,
            hostname,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn bucket_id_for(client_name: &str, hostname: &str) -> String {
        format!("{}_{}", client_name, hostname)
    }

    /// Wait until aw-server responds, then create the bucket (idempotent).
    pub async fn wait_and_create_bucket(
        &self,
        bucket_id: &str,
        client_name: &str,
        event_type: &str,
    ) -> Result<(), reqwest::Error> {
        let bucket_url = format!("{}/api/0/buckets/{}", self.base_url, bucket_id);
        let info = BucketInfo {
            client: client_name,
            hostname: &self.hostname,
            bucket_type: event_type,
        };

        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            match self.http.post(&bucket_url).json(&info).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() || status.as_u16() == 304 {
                        info!(
                            "Bucket {} ready (status {}, type={}, client={})",
                            bucket_id, status, event_type, client_name
                        );
                        return Ok(());
                    }
                    warn!(
                        "Bucket create returned {}; will retry (attempt {})",
                        status, attempt
                    );
                }
                Err(e) => {
                    if attempt == 1 || attempt.is_multiple_of(10) {
                        warn!(
                            "Waiting for aw-server at {} (attempt {}): {}",
                            self.base_url, attempt, e
                        );
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    pub async fn heartbeat<T: Serialize + std::fmt::Debug>(
        &self,
        bucket_id: &str,
        data: T,
        pulsetime: f64,
    ) {
        let event = Event {
            timestamp: Utc::now(),
            duration: 0.0,
            data,
        };

        let url = format!(
            "{}/api/0/buckets/{}/heartbeat?pulsetime={}",
            self.base_url, bucket_id, pulsetime
        );

        match self.http.post(&url).json(&event).send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!("Heartbeat ok [{}]: {:?}", bucket_id, event.data);
            }
            Ok(resp) => {
                warn!(
                    "Heartbeat non-success status [{}]: {}",
                    bucket_id,
                    resp.status()
                );
            }
            Err(e) => {
                warn!("Failed to send heartbeat [{}]: {}", bucket_id, e);
            }
        }
    }
}
