use crate::types::Availability;
use reqwest::{Client, StatusCode};
use std::time::Duration;

pub async fn check_rdap(
    client: &Client,
    endpoint: &str,
    domain: &str,
    timeout: Duration,
) -> Availability {
    let url = format!("{}/domain/{}", endpoint, domain);
    
    let result = tokio::time::timeout(timeout, client.get(&url).send()).await;
    
    match result {
        Ok(Ok(response)) => match response.status() {
            StatusCode::NOT_FOUND => Availability::Available,
            StatusCode::OK => Availability::Taken,
            StatusCode::TOO_MANY_REQUESTS => {
                Availability::Unknown { reason: "Rate limited".to_string() }
            }
            status => Availability::Unknown {
                reason: format!("HTTP {}", status.as_u16()),
            },
        },
        Ok(Err(e)) => Availability::Unknown {
            reason: format!("Request failed: {}", e),
        },
        Err(_) => Availability::Unknown {
            reason: "Timeout".to_string(),
        },
    }
}
