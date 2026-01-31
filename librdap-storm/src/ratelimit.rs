use dashmap::DashMap;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::{num::NonZeroU32, sync::Arc};

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

pub struct EndpointRateLimiters {
    limiters: DashMap<String, Arc<Limiter>>,
    default_rate: u32,
}

impl EndpointRateLimiters {
    pub fn new(default_rate_per_second: u32) -> Self {
        Self {
            limiters: DashMap::new(),
            default_rate: default_rate_per_second,
        }
    }

    pub async fn acquire(&self, endpoint: &str) {
        let limiter = self.get_or_create(endpoint);
        limiter.until_ready().await;
    }

    fn get_or_create(&self, endpoint: &str) -> Arc<Limiter> {
        self.limiters
            .entry(endpoint.to_string())
            .or_insert_with(|| {
                let quota = Quota::per_second(NonZeroU32::new(self.default_rate).unwrap());
                Arc::new(RateLimiter::direct(quota))
            })
            .clone()
    }

}
