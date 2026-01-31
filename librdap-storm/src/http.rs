use reqwest::Client;
use std::time::Duration;

pub fn create_http_pool(timeout: Duration) -> Client {
    Client::builder()
        .timeout(timeout)
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .tcp_nodelay(true)
        .use_rustls_tls()
        .build()
        .expect("Failed to create HTTP client")
}
