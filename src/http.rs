#![allow(dead_code)]

use std::{sync::OnceLock, time::Duration};

use anyhow::{bail, Context};
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    StatusCode,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ATTEMPTS: u32 = 3;
const FIRST_BACKOFF: Duration = Duration::from_millis(400);
const ERROR_BODY_LIMIT: usize = 300;

static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn user_agent() -> String {
    format!("RendogLauncher/{}", env!("CARGO_PKG_VERSION"))
}

pub fn client() -> anyhow::Result<Client> {
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }

    let client = Client::builder()
        .user_agent(user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("네트워크 클라이언트를 준비하지 못했어요.")?;

    let _ = CLIENT.set(client.clone());

    Ok(client)
}

/// Send a request, retrying transient failures with exponential backoff.
///
/// Only transport errors and the statuses in [`is_retryable`] are retried; a
/// 4xx means the request itself is wrong and repeating it changes nothing.
pub fn send(request: RequestBuilder) -> anyhow::Result<Response> {
    check(send_raw(request)?)
}

/// Like [`send`], but hands back non-success responses instead of turning them
/// into errors. Needed where the status is part of the protocol — OAuth device
/// polling answers `400 authorization_pending` on every tick until the user is
/// done.
pub fn send_raw(request: RequestBuilder) -> anyhow::Result<Response> {
    let mut backoff = FIRST_BACKOFF;
    let mut last: Option<anyhow::Error> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let Some(pending) = request.try_clone() else {
            return request.send().context("네트워크 요청에 실패했어요.");
        };

        match pending.send() {
            Ok(response) if !is_retryable(response.status()) => return Ok(response),
            Ok(response) if attempt == MAX_ATTEMPTS => return Ok(response),
            Ok(response) => {
                log::warn!("요청 재시도 {attempt}/{MAX_ATTEMPTS} (HTTP {})", response.status());
            }
            Err(error) if attempt == MAX_ATTEMPTS => {
                last = Some(anyhow::Error::new(error));
            }
            Err(error) => {
                log::warn!("요청 재시도 {attempt}/{MAX_ATTEMPTS} ({error})");
                last = Some(anyhow::Error::new(error));
            }
        }

        if attempt < MAX_ATTEMPTS {
            std::thread::sleep(backoff);
            backoff = next_backoff(backoff);
        }
    }

    Err(last.unwrap_or_else(|| anyhow::anyhow!("네트워크 요청에 실패했어요.")))
        .context("네트워크 연결을 확인한 뒤 다시 시도해 주세요.")
}

fn check(response: Response) -> anyhow::Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let url = response.url().clone();
    let body = response.text().unwrap_or_default();

    bail!(
        "서버가 HTTP {}를 반환했어요. ({}){}",
        status.as_u16(),
        url,
        snippet(&body)
    )
}

fn snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut end = trimmed.len().min(ERROR_BODY_LIMIT);
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }

    format!(" {}", &trimmed[..end])
}

fn is_retryable(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
}

fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_transient_statuses() {
        for code in [500, 502, 503, 504, 408, 429] {
            assert!(
                is_retryable(StatusCode::from_u16(code).unwrap()),
                "HTTP {code} should be retried"
            );
        }

        for code in [200, 204, 301, 400, 401, 403, 404] {
            assert!(
                !is_retryable(StatusCode::from_u16(code).unwrap()),
                "HTTP {code} must not be retried"
            );
        }
    }

    #[test]
    fn backoff_doubles_and_stays_bounded_by_attempts() {
        let mut delay = FIRST_BACKOFF;
        let mut total = Duration::ZERO;

        for _ in 1..MAX_ATTEMPTS {
            total += delay;
            delay = next_backoff(delay);
        }

        assert_eq!(total, Duration::from_millis(400 + 800));
    }

    #[test]
    fn error_snippet_is_truncated_on_a_character_boundary() {
        let body = "다".repeat(500);
        let cut = snippet(&body);

        assert!(cut.len() <= ERROR_BODY_LIMIT + 1);
        assert!(cut.trim_start().chars().all(|c| c == '다'));
    }

    #[test]
    fn empty_bodies_add_nothing() {
        assert_eq!(snippet("   "), "");
    }

    #[test]
    fn user_agent_carries_the_version() {
        assert_eq!(user_agent(), format!("RendogLauncher/{}", env!("CARGO_PKG_VERSION")));
    }
}
