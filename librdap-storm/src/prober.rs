use crate::{
    endpoint::{extract_tld, EndpointRegistry},
    http::create_http_pool,
    ratelimit::EndpointRateLimiters,
    rdap::check_rdap,
    types::{Availability, ProbeConfig, ProbeResult},
    whois::check_whois,
};
use futures::stream::{self, Stream, StreamExt};
use reqwest::Client;
use std::{sync::Arc, time::Instant};

pub struct Prober {
    client: Client,
    registry: Arc<EndpointRegistry>,
    rate_limiters: Arc<EndpointRateLimiters>,
    config: ProbeConfig,
}

impl Prober {
    pub fn new() -> Self {
        Self::with_config(ProbeConfig::default())
    }

    pub fn with_config(config: ProbeConfig) -> Self {
        let client = create_http_pool(config.timeout);
        Self {
            client,
            registry: Arc::new(EndpointRegistry::new()),
            rate_limiters: Arc::new(EndpointRateLimiters::new(config.max_rate_per_endpoint)),
            config,
        }
    }

    pub async fn ensure_bootstrapped(&self) -> Result<(), crate::endpoint::EndpointError> {
        self.registry.bootstrap(&self.client).await
    }

    pub async fn probe_one(&self, domain: &str) -> ProbeResult {
        let start = Instant::now();
        
        if let Err(e) = self.ensure_bootstrapped().await {
            return ProbeResult {
                domain: domain.to_string(),
                availability: Availability::Unknown { reason: format!("Bootstrap failed: {}", e) },
                duration: start.elapsed(),
            };
        }

        let tld = match extract_tld(domain) {
            Ok(t) => t,
            Err(e) => {
                return ProbeResult {
                    domain: domain.to_string(),
                    availability: Availability::Unknown { reason: e.to_string() },
                    duration: start.elapsed(),
                };
            }
        };

        let endpoint = match self.registry.get_endpoint(&tld) {
            Some(e) => e,
            None => {
                if self.config.whois_fallback {
                    let availability = check_whois(domain, self.config.timeout).await;
                    return ProbeResult {
                        domain: domain.to_string(),
                        availability,
                        duration: start.elapsed(),
                    };
                }
                return ProbeResult {
                    domain: domain.to_string(),
                    availability: Availability::Unknown { 
                        reason: format!("No RDAP endpoint for .{}", tld) 
                    },
                    duration: start.elapsed(),
                };
            }
        };

        self.rate_limiters.acquire(&endpoint).await;

        let availability = check_rdap(&self.client, &endpoint, domain, self.config.timeout).await;

        let availability = if matches!(availability, Availability::Unknown { .. }) && self.config.whois_fallback {
            check_whois(domain, self.config.timeout).await
        } else {
            availability
        };

        ProbeResult {
            domain: domain.to_string(),
            availability,
            duration: start.elapsed(),
        }
    }

    pub fn probe_stream<I>(&self, domains: I) -> impl Stream<Item = ProbeResult> + '_
    where
        I: IntoIterator<Item = String> + 'static,
    {
        let domains: Vec<String> = domains.into_iter().collect();
        
        stream::iter(domains)
            .map(move |domain| async move {
                self.probe_one(&domain).await
            })
            .buffer_unordered(self.config.max_concurrent_per_endpoint as usize * 10)
    }
}

impl Default for Prober {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Prober {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            registry: Arc::clone(&self.registry),
            rate_limiters: Arc::clone(&self.rate_limiters),
            config: self.config.clone(),
        }
    }
}
