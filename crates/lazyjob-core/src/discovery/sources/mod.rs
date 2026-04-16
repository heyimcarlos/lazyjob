pub mod greenhouse;
pub mod lever;

pub use greenhouse::GreenhouseClient;
pub use lever::LeverClient;

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::domain::Job;
use crate::error::Result;

#[async_trait]
pub trait JobSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn fetch_jobs(&self, company_id: &str) -> Result<Vec<Job>>;
}

pub struct RateLimiter {
    interval: Duration,
    last_call: Mutex<Option<Instant>>,
}

impl RateLimiter {
    pub fn new(requests_per_second: u32) -> Self {
        let nanos = 1_000_000_000u64 / requests_per_second.max(1) as u64;
        Self {
            interval: Duration::from_nanos(nanos),
            last_call: Mutex::new(None),
        }
    }

    pub async fn wait(&self) {
        let sleep_duration = {
            let mut guard = self.last_call.lock().expect("rate limiter mutex poisoned");
            let now = Instant::now();
            let sleep = match *guard {
                Some(last) => {
                    let elapsed = now.duration_since(last);
                    if elapsed < self.interval {
                        Some(self.interval - elapsed)
                    } else {
                        None
                    }
                }
                None => None,
            };
            *guard = Some(now);
            sleep
        };

        if let Some(dur) = sleep_duration {
            tokio::time::sleep(dur).await;
        }
    }
}

pub(crate) fn strip_html(html: &str) -> String {
    ammonia::Builder::new()
        .tags(HashSet::new())
        .clean(html)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies ammonia strips all HTML tags and keeps text content
    #[test]
    fn ammonia_strips_html_tags() {
        let result = ammonia::Builder::new()
            .tags(HashSet::new())
            .clean("<p>Hello <b>World</b></p>")
            .to_string();
        assert_eq!(result.trim(), "Hello World");
    }

    #[test]
    fn strip_html_empty_string() {
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn strip_html_plain_text() {
        assert_eq!(strip_html("hello world"), "hello world");
    }

    #[test]
    fn strip_html_nested_tags() {
        let result = strip_html("<div><p>Test <b>bold</b> text</p></div>");
        assert!(result.contains("Test"));
        assert!(result.contains("bold"));
        assert!(result.contains("text"));
        assert!(!result.contains('<'));
    }

    #[test]
    fn strip_html_removes_attributes() {
        let result = strip_html(r#"<p class="foo" id="bar">Content</p>"#);
        assert_eq!(result.trim(), "Content");
    }

    #[test]
    fn rate_limiter_allows_first_call_immediately() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let limiter = RateLimiter::new(1);
        let start = Instant::now();
        rt.block_on(limiter.wait());
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
