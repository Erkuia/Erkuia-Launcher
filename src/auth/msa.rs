use std::time::{Duration, Instant};

use anyhow::Context;
use serde::Deserialize;

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

/// Hand the URL to the shell so it lands in whatever browser the user has set.
pub fn open_in_browser(url: &str) -> anyhow::Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(url)
        .spawn()
        .with_context(|| format!("브라우저를 열지 못했어요. 직접 접속해 주세요: {url}"))?;

    Ok(())
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
    fn expiry_is_reported() {
        let mut raw = response();
        raw.expires_in = 0;

        let code = DeviceCode::from_response(raw, Instant::now());

        assert!(code.is_expired());
    }
}
