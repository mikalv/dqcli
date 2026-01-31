use dashmap::DashMap;
use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

const IANA_BOOTSTRAP_URL: &str = "https://data.iana.org/rdap/dns.json";

#[derive(Debug, Error)]
pub enum EndpointError {
    #[error("Failed to fetch IANA bootstrap: {0}")]
    FetchError(#[from] reqwest::Error),
    #[error("No RDAP endpoint found for TLD: {0}")]
    NoEndpoint(String),
    #[error("Invalid domain format: {0}")]
    InvalidDomain(String),
}

#[derive(Debug, Deserialize)]
struct IanaBootstrap {
    services: Vec<(Vec<String>, Vec<String>)>,
}

pub struct EndpointRegistry {
    endpoints: DashMap<String, String>,
    bootstrapped: std::sync::atomic::AtomicBool,
}

impl EndpointRegistry {
    pub fn new() -> Self {
        Self {
            endpoints: DashMap::new(),
            bootstrapped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub async fn bootstrap(&self, client: &Client) -> Result<(), EndpointError> {
        if self.bootstrapped.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        let resp: IanaBootstrap = client
            .get(IANA_BOOTSTRAP_URL)
            .send()
            .await?
            .json()
            .await?;

        for (tlds, urls) in resp.services {
            if let Some(url) = urls.first() {
                let base_url = url.trim_end_matches('/').to_string();
                for tld in tlds {
                    self.endpoints.insert(tld.to_lowercase(), base_url.clone());
                }
            }
        }

        self.bootstrapped.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub fn get_endpoint(&self, tld: &str) -> Option<String> {
        self.endpoints.get(&tld.to_lowercase()).map(|v| v.clone())
    }

}

impl Default for EndpointRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn extract_tld(domain: &str) -> Result<String, EndpointError> {
    domain
        .rsplit('.')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .ok_or_else(|| EndpointError::InvalidDomain(domain.to_string()))
}
