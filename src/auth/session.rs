use std::time::{Duration, Instant};

use anyhow::{bail, Context};

use crate::auth::{
    device::{self, DeviceIdentity, DeviceToken},
    minecraft::{self, MinecraftProfile, MinecraftToken},
    msa::MsaToken,
    sisu,
    xbox::{XblToken, JAVA_XSTS_RELYING_PARTY},
};

const CLOCK_SKEW: Duration = Duration::from_secs(60);
const DEVICE_TOKEN_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const XSTS_TTL: Duration = Duration::from_secs(8 * 60 * 60);

struct Expiring<T> {
    value: T,
    expires_at: Instant,
}

impl<T> Expiring<T> {
    fn new(value: T, ttl: Duration) -> Self {
        Self {
            value,
            expires_at: Instant::now() + ttl,
        }
    }

    fn until(value: T, expires_at: Instant) -> Self {
        Self { value, expires_at }
    }

    fn valid(&self) -> bool {
        Instant::now() + CLOCK_SKEW < self.expires_at
    }
}

fn take_valid<T>(slot: &mut Option<Expiring<T>>) -> Option<&T> {
    if slot.as_ref().is_some_and(Expiring::valid) {
        return slot.as_ref().map(|held| &held.value);
    }

    *slot = None;
    None
}

pub struct Session {
    identity: DeviceIdentity,
    refresh_token: String,
    msa: Option<Expiring<MsaToken>>,
    device: Option<Expiring<DeviceToken>>,
    xsts: Option<Expiring<XblToken>>,
    minecraft: Option<Expiring<MinecraftToken>>,
    profile: Option<MinecraftProfile>,
}

impl Session {
    pub fn from_msa_token(identity: DeviceIdentity, token: MsaToken) -> anyhow::Result<Self> {
        let Some(refresh_token) = token.refresh_token.clone() else {
            bail!("리프레시 토큰이 없어 로그인을 저장할 수 없어요.");
        };

        let expires_at = token.expires_at;

        Ok(Self {
            identity,
            refresh_token,
            msa: Some(Expiring::until(token, expires_at)),
            device: None,
            xsts: None,
            minecraft: None,
            profile: None,
        })
    }

    pub fn from_refresh_token(identity: DeviceIdentity, refresh_token: String) -> Self {
        Self {
            identity,
            refresh_token,
            msa: None,
            device: None,
            xsts: None,
            minecraft: None,
            profile: None,
        }
    }

    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    pub fn cached_profile(&self) -> Option<&MinecraftProfile> {
        self.profile.as_ref()
    }

    fn access_token(&mut self) -> anyhow::Result<String> {
        if let Some(token) = take_valid(&mut self.msa) {
            return Ok(token.access_token.clone());
        }

        let token = crate::auth::msa::refresh(&self.refresh_token)?;

        if let Some(rotated) = token.refresh_token.clone() {
            self.refresh_token = rotated;
        }

        self.invalidate_downstream();

        let expires_at = token.expires_at;
        let access = token.access_token.clone();
        self.msa = Some(Expiring::until(token, expires_at));

        Ok(access)
    }

    fn device_token(&mut self) -> anyhow::Result<DeviceToken> {
        if let Some(token) = take_valid(&mut self.device) {
            return Ok(token.clone());
        }

        let token = device::authenticate(&self.identity)?;
        let value = token.clone();
        self.device = Some(Expiring::new(token, DEVICE_TOKEN_TTL));

        Ok(value)
    }

    fn xsts_token(&mut self) -> anyhow::Result<XblToken> {
        if let Some(token) = take_valid(&mut self.xsts) {
            return Ok(token.clone());
        }

        let access = self.access_token()?;
        let device_token = self.device_token()?;

        let tokens = sisu::authorize(
            &access,
            &device_token,
            &self.identity,
            JAVA_XSTS_RELYING_PARTY,
        )?;

        self.xsts = Some(Expiring::new(tokens.authorization.clone(), XSTS_TTL));

        Ok(tokens.authorization)
    }

    pub fn minecraft_token(&mut self) -> anyhow::Result<MinecraftToken> {
        if let Some(token) = take_valid(&mut self.minecraft) {
            return Ok(token.clone());
        }

        let xsts = self.xsts_token()?;
        let token = minecraft::login(&xsts)?;
        let expires_at = token.expires_at;
        let value = token.clone();
        self.minecraft = Some(Expiring::until(token, expires_at));

        Ok(value)
    }

    pub fn verify_ownership(&mut self) -> anyhow::Result<()> {
        let token = self.minecraft_token()?;
        let items = minecraft::entitlements(&token)?;

        if !minecraft::owns_java_edition(&items) {
            bail!("이 계정은 Minecraft Java Edition을 소유하고 있지 않아요.");
        }

        Ok(())
    }

    pub fn profile(&mut self) -> anyhow::Result<MinecraftProfile> {
        if let Some(profile) = self.profile.clone() {
            return Ok(profile);
        }

        let token = self.minecraft_token()?;
        let profile = minecraft::profile(&token).context("프로필을 확인하지 못했어요.")?;

        self.profile = Some(profile.clone());

        Ok(profile)
    }

    fn invalidate_downstream(&mut self) {
        self.xsts = None;
        self.minecraft = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> DeviceIdentity {
        DeviceIdentity::generate()
    }

    fn msa(expires_in: Duration, refresh: Option<&str>) -> MsaToken {
        MsaToken {
            access_token: "ACCESS".to_string(),
            refresh_token: refresh.map(str::to_string),
            expires_at: Instant::now() + expires_in,
        }
    }

    #[test]
    fn a_login_without_a_refresh_token_is_rejected() {
        let result = Session::from_msa_token(identity(), msa(Duration::from_secs(3600), None));

        assert!(result.is_err());
    }

    #[test]
    fn a_fresh_login_keeps_its_refresh_token() {
        let session =
            Session::from_msa_token(identity(), msa(Duration::from_secs(3600), Some("RT"))).unwrap();

        assert_eq!(session.refresh_token(), "RT");
    }

    #[test]
    fn a_restored_session_starts_with_nothing_cached() {
        let session = Session::from_refresh_token(identity(), "RT".to_string());

        assert!(session.msa.is_none());
        assert!(session.device.is_none());
        assert!(session.xsts.is_none());
        assert!(session.minecraft.is_none());
        assert!(session.cached_profile().is_none());
    }

    #[test]
    fn a_token_inside_its_window_is_reused() {
        let mut slot = Some(Expiring::new("value", Duration::from_secs(3600)));

        assert_eq!(take_valid(&mut slot), Some(&"value"));
        assert!(slot.is_some());
    }

    #[test]
    fn an_expired_token_is_dropped() {
        let mut slot = Some(Expiring {
            value: "value",
            expires_at: Instant::now(),
        });

        assert_eq!(take_valid(&mut slot), None);
        assert!(slot.is_none());
    }

    #[test]
    fn a_token_expiring_within_the_skew_is_treated_as_expired() {
        let mut slot = Some(Expiring::new("value", CLOCK_SKEW / 2));

        assert_eq!(take_valid(&mut slot), None);
    }

    #[test]
    fn a_token_beyond_the_skew_is_still_usable() {
        let mut slot = Some(Expiring::new("value", CLOCK_SKEW * 2));

        assert_eq!(take_valid(&mut slot), Some(&"value"));
    }

    #[test]
    fn refreshing_the_msa_token_drops_everything_derived_from_it() {
        let mut session =
            Session::from_msa_token(identity(), msa(Duration::from_secs(3600), Some("RT"))).unwrap();
        session.xsts = Some(Expiring::new(
            XblToken {
                token: "X".to_string(),
                user_hash: "U".to_string(),
                not_after: String::new(),
            },
            Duration::from_secs(3600),
        ));
        session.minecraft = Some(Expiring::new(
            MinecraftToken {
                access_token: "M".to_string(),
                token_type: "Bearer".to_string(),
                expires_at: Instant::now() + Duration::from_secs(3600),
            },
            Duration::from_secs(3600),
        ));

        session.invalidate_downstream();

        assert!(session.xsts.is_none());
        assert!(session.minecraft.is_none());
    }

    #[test]
    fn the_device_token_outlives_an_msa_refresh() {
        let mut session = Session::from_refresh_token(identity(), "RT".to_string());
        session.device = Some(Expiring::new(
            DeviceToken {
                token: "D".to_string(),
                device_id: "id".to_string(),
                not_after: String::new(),
            },
            DEVICE_TOKEN_TTL,
        ));

        session.invalidate_downstream();

        assert!(session.device.is_some());
    }

    #[test]
    fn the_cached_windows_are_ordered_by_how_long_each_token_lives() {
        assert!(DEVICE_TOKEN_TTL > XSTS_TTL);
        assert!(XSTS_TTL > CLOCK_SKEW);
    }
}
