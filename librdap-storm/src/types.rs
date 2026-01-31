use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    Taken,
    Unknown { reason: String },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }

    pub fn is_taken(&self) -> bool {
        matches!(self, Availability::Taken)
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Availability::Unknown { .. })
    }
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub domain: String,
    pub availability: Availability,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct ProbeConfig {
    pub timeout: Duration,
    pub whois_fallback: bool,
    pub max_rate_per_endpoint: u32,
    pub max_concurrent_per_endpoint: u32,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            whois_fallback: true,
            max_rate_per_endpoint: 20,
            max_concurrent_per_endpoint: 10,
        }
    }
}
