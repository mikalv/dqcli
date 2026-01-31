mod endpoint;
mod http;
mod prober;
mod ratelimit;
mod rdap;
pub mod tlds;
mod types;
mod whois;

pub use prober::Prober;
pub use types::{Availability, ProbeConfig, ProbeResult};
pub use tlds::{expand_tlds, fetch_iana_tlds};

use futures::StreamExt;

pub async fn probe(domain: &str) -> ProbeResult {
    Prober::new().probe_one(domain).await
}

pub async fn probe_many<I>(domains: I) -> Vec<ProbeResult>
where
    I: IntoIterator<Item = String> + 'static,
{
    Prober::new().probe_stream(domains).collect().await
}
