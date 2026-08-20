use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::task::Cancel;

/// Official Minecraft Java launcher title id. Using it means no Azure app
/// registration is needed. Mirrors `MsaConstants.JAVA_TITLE_ID`.
pub const CLIENT_ID: &str = "00000000402b5328";

/// `MsaConstants.SCOPE_TITLE_AUTH`. Title auth issues a refresh token without
/// asking for `offline_access`.
pub const SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";

/// Title auth lives on the legacy live.com endpoints, not the v2.0 ones.
pub const DEVICE_CODE_URL: &str = "https://login.live.com/oauth20_connect.srf";
pub const TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";

const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(5);
const MAX_LOGIN_WINDOW: Duration = Duration::from_secs(300);
const SLOW_DOWN_STEP_SECS: u64 = 5;

#[derive(Debug, Clone, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default)]
    interval: u64,
}

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: Duration,
    pub expires_at: Instant,
}

impl DeviceCode {
    fn from_response(response: DeviceCodeResponse, now: Instant) -> Self {
        let interval = match response.interval {
            0 => FALLBACK_POLL_INTERVAL,
            seconds => Duration::from_secs(seconds).max(MIN_POLL_INTERVAL),
        };

        // Microsoft hands out a long window, but a login left open for that long
        // is abandoned rather than pending.
        let lifetime = Duration::from_secs(response.expires_in).min(MAX_LOGIN_WINDOW);

        Self {
            device_code: response.device_code,
            user_code: response.user_code,
            verification_uri: response.verification_uri,
            interval,
            expires_at: now + lifetime,
        }
    }

    /// URL with the code already filled in, so the user only has to confirm.
    /// Mirrors `MsaDeviceCode.getDirectVerificationUri()`.
    pub fn direct_verification_uri(&self) -> String {
        format!("{}?otc={}", self.verification_uri, self.user_code)
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

pub fn request_device_code() -> anyhow::Result<DeviceCode> {
    let response = crate::http::send(
        crate::http::client()?
            .post(DEVICE_CODE_URL)
            .form(&[
                ("client_id", CLIENT_ID),
                ("scope", SCOPE),
                ("response_type", "device_code"),
            ]),
    )
    .context("Microsoft 로그인 코드를 받지 못했어요.")?;

    let parsed: DeviceCodeResponse = response
        .json()
        .context("Microsoft 로그인 응답을 해석하지 못했어요.")?;

    log::info!("디바이스 코드 발급됨 (user_code={})", parsed.user_code);

    Ok(DeviceCode::from_response(parsed, Instant::now()))
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MsaToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Instant,
}

/// What a non-success poll response means for the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollVerdict {
    KeepWaiting,
    SlowDown,
    Give(&'static str),
}

fn classify(error: &str) -> PollVerdict {
    match error {
        "authorization_pending" => PollVerdict::KeepWaiting,
        "slow_down" => PollVerdict::SlowDown,
        "authorization_declined" | "access_denied" => {
            PollVerdict::Give("로그인이 거부됐어요. 다시 시도해 주세요.")
        }
        "expired_token" | "code_expired" => {
            PollVerdict::Give("로그인 시간이 만료됐어요. 다시 시도해 주세요.")
        }
        "bad_verification_code" | "invalid_grant" => {
            PollVerdict::Give("로그인 코드가 올바르지 않아요. 다시 시도해 주세요.")
        }
        _ => PollVerdict::Give("Microsoft 로그인에 실패했어요."),
    }
}

/// Poll until the user finishes in the browser.
///
/// The endpoint answers `400 authorization_pending` on every tick until then,
/// so a non-success status here is normal traffic rather than a failure.
pub fn poll_for_token(code: &DeviceCode, cancel: &Cancel) -> anyhow::Result<MsaToken> {
    let mut interval = code.interval;

    loop {
        if cancel.is_cancelled() {
            bail!("로그인을 취소했어요.");
        }

        if code.is_expired() {
            bail!("로그인 시간이 만료됐어요. 다시 시도해 주세요.");
        }

        let response = crate::http::send_raw(
            crate::http::client()?
                .post(TOKEN_URL)
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("grant_type", "device_code"),
                    ("device_code", code.device_code.as_str()),
                ]),
        )
        .context("Microsoft 로그인 상태를 확인하지 못했어요.")?;

        if response.status().is_success() {
            let token: TokenResponse = response
                .json()
                .context("Microsoft 로그인 응답을 해석하지 못했어요.")?;

            log::info!("Microsoft 토큰 발급 완료");

            return Ok(MsaToken {
                access_token: token.access_token,
                refresh_token: token.refresh_token,
                expires_at: Instant::now() + Duration::from_secs(token.expires_in),
            });
        }

        let status = response.status();
        let failure: ErrorResponse = response.json().unwrap_or(ErrorResponse {
            error: format!("http_{}", status.as_u16()),
            error_description: None,
        });

        match classify(&failure.error) {
            PollVerdict::KeepWaiting => {}
            PollVerdict::SlowDown => {
                interval = interval.saturating_add(Duration::from_secs(SLOW_DOWN_STEP_SECS));
                log::warn!("폴링 간격을 {}초로 늘립니다.", interval.as_secs());
            }
            PollVerdict::Give(message) => {
                log::error!(
                    "로그인 실패: {} ({})",
                    failure.error,
                    failure.error_description.unwrap_or_default()
                );
                bail!("{message}");
            }
        }

        if !cancel.sleep(interval) {
            bail!("로그인을 취소했어요.");
        }
    }
}

pub fn refresh(refresh_token: &str) -> anyhow::Result<MsaToken> {
    let response = crate::http::send_raw(
        crate::http::client()?
            .post(TOKEN_URL)
            .form(&[
                ("client_id", CLIENT_ID),
                ("scope", SCOPE),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ]),
    )
    .context("Microsoft 세션을 갱신하지 못했어요.")?;

    if !response.status().is_success() {
        let status = response.status();
        let failure: ErrorResponse = response.json().unwrap_or(ErrorResponse {
            error: format!("http_{}", status.as_u16()),
            error_description: None,
        });

        log::error!(
            "세션 갱신 실패: {} ({})",
            failure.error,
            failure.error_description.unwrap_or_default()
        );

        bail!("저장된 로그인이 만료됐어요. 다시 로그인해 주세요.");
    }

    let token: TokenResponse = response
        .json()
        .context("Microsoft 갱신 응답을 해석하지 못했어요.")?;

    log::info!("Microsoft 세션 갱신 완료");

    Ok(MsaToken {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: Instant::now() + Duration::from_secs(token.expires_in),
    })
}

/// Hand the URL to the shell so it lands in whatever browser the user has set.
pub fn open_in_browser(url: &str) -> anyhow::Result<()> {
    crate::shell::open(url)
        .with_context(|| format!("브라우저를 열지 못했어요. 직접 접속해 주세요: {url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> DeviceCodeResponse {
        DeviceCodeResponse {
            device_code: "DEV".to_string(),
            user_code: "ABCD1234".to_string(),
            verification_uri: "https://www.microsoft.com/link".to_string(),
            expires_in: 900,
            interval: 5,
        }
    }

    #[test]
    fn parses_the_live_endpoint_shape() {
        let json = r#"{
            "user_code": "ABCD1234",
            "device_code": "DEV",
            "verification_uri": "https://www.microsoft.com/link",
            "expires_in": 900,
            "interval": 5
        }"#;

        let parsed: DeviceCodeResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.user_code, "ABCD1234");
        assert_eq!(parsed.device_code, "DEV");
        assert_eq!(parsed.interval, 5);
    }

    #[test]
    fn builds_the_prefilled_url() {
        let code = DeviceCode::from_response(response(), Instant::now());

        assert_eq!(
            code.direct_verification_uri(),
            "https://www.microsoft.com/link?otc=ABCD1234"
        );
    }

    #[test]
    fn a_missing_interval_falls_back_instead_of_hammering_the_endpoint() {
        let mut raw = response();
        raw.interval = 0;

        let code = DeviceCode::from_response(raw, Instant::now());

        assert_eq!(code.interval, FALLBACK_POLL_INTERVAL);
    }

    #[test]
    fn poll_interval_never_drops_below_one_second() {
        let mut raw = response();
        raw.interval = 1;

        let code = DeviceCode::from_response(raw, Instant::now());

        assert!(code.interval >= MIN_POLL_INTERVAL);
    }

    #[test]
    fn login_window_is_capped() {
        let mut raw = response();
        raw.expires_in = 86_400;

        let now = Instant::now();
        let code = DeviceCode::from_response(raw, now);

        assert_eq!(code.expires_at - now, MAX_LOGIN_WINDOW);
    }

    #[test]
    fn a_short_window_is_kept_as_is() {
        let mut raw = response();
        raw.expires_in = 120;

        let now = Instant::now();
        let code = DeviceCode::from_response(raw, now);

        assert_eq!(code.expires_at - now, Duration::from_secs(120));
    }

    #[test]
    fn pending_and_slow_down_keep_the_loop_alive() {
        assert_eq!(classify("authorization_pending"), PollVerdict::KeepWaiting);
        assert_eq!(classify("slow_down"), PollVerdict::SlowDown);
    }

    #[test]
    fn terminal_errors_stop_the_loop() {
        for error in [
            "authorization_declined",
            "access_denied",
            "expired_token",
            "code_expired",
            "bad_verification_code",
            "invalid_grant",
            "something_new_from_microsoft",
        ] {
            assert!(
                matches!(classify(error), PollVerdict::Give(_)),
                "{error} should stop polling"
            );
        }
    }

    #[test]
    fn declined_and_expired_get_distinct_messages() {
        let declined = classify("authorization_declined");
        let expired = classify("expired_token");

        assert_ne!(declined, expired);
    }

    #[test]
    fn parses_a_successful_token_response() {
        let json = r#"{
            "token_type": "bearer",
            "expires_in": 86400,
            "access_token": "AT",
            "refresh_token": "RT"
        }"#;

        let parsed: TokenResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.access_token, "AT");
        assert_eq!(parsed.refresh_token.as_deref(), Some("RT"));
        assert_eq!(parsed.expires_in, 86_400);
    }

    #[test]
    fn a_token_response_without_a_refresh_token_still_parses() {
        let parsed: TokenResponse =
            serde_json::from_str(r#"{"access_token":"AT","expires_in":3600}"#).unwrap();

        assert_eq!(parsed.refresh_token, None);
    }

    #[test]
    fn parses_the_pending_error_body() {
        let json = r#"{"error":"authorization_pending","error_description":"waiting"}"#;

        let parsed: ErrorResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.error, "authorization_pending");
        assert_eq!(parsed.error_description.as_deref(), Some("waiting"));
    }

    #[test]
    fn expiry_is_reported() {
        let mut raw = response();
        raw.expires_in = 0;

        let code = DeviceCode::from_response(raw, Instant::now());

        assert!(code.is_expired());
    }

    #[test]
    fn polling_stops_immediately_when_cancelled() {
        let code = DeviceCode::from_response(response(), Instant::now());
        let cancel = Cancel::new();
        cancel.cancel();

        let started = Instant::now();
        let error = poll_for_token(&code, &cancel).unwrap_err();

        assert!(error.to_string().contains("취소"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn polling_stops_when_the_code_has_already_expired() {
        let mut raw = response();
        raw.expires_in = 0;
        let code = DeviceCode::from_response(raw, Instant::now());

        let error = poll_for_token(&code, &Cancel::new()).unwrap_err();

        assert!(error.to_string().contains("만료"));
    }
}
