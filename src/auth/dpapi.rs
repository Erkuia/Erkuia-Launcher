pub const ENTROPY: &[u8] = b"ErkuiaLauncher/accounts/v1";

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use anyhow::bail;

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    impl DataBlob {
        fn borrowed(bytes: &[u8]) -> Self {
            Self {
                cb_data: bytes.len() as u32,
                pb_data: bytes.as_ptr() as *mut u8,
            }
        }

        fn empty() -> Self {
            Self {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            }
        }
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(handle: *mut c_void) -> *mut c_void;
    }

    fn take(blob: DataBlob) -> Vec<u8> {
        if blob.pb_data.is_null() {
            return Vec::new();
        }

        let bytes =
            unsafe { std::slice::from_raw_parts(blob.pb_data, blob.cb_data as usize) }.to_vec();
        unsafe { LocalFree(blob.pb_data as *mut c_void) };

        bytes
    }

    pub fn protect(plain: &[u8], entropy: &[u8]) -> anyhow::Result<Vec<u8>> {
        let input = DataBlob::borrowed(plain);
        let salt = DataBlob::borrowed(entropy);
        let mut output = DataBlob::empty();

        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &salt,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };

        if ok == 0 {
            bail!(
                "계정 정보를 암호화하지 못했어요. (오류 {})",
                std::io::Error::last_os_error()
            );
        }

        Ok(take(output))
    }

    pub fn unprotect(cipher: &[u8], entropy: &[u8]) -> anyhow::Result<Vec<u8>> {
        let input = DataBlob::borrowed(cipher);
        let salt = DataBlob::borrowed(entropy);
        let mut output = DataBlob::empty();

        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &salt,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };

        if ok == 0 {
            bail!(
                "계정 정보를 복호화하지 못했어요. 다시 로그인해 주세요. (오류 {})",
                std::io::Error::last_os_error()
            );
        }

        Ok(take(output))
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::bail;

    pub fn protect(_plain: &[u8], _entropy: &[u8]) -> anyhow::Result<Vec<u8>> {
        bail!("계정 암호화는 Windows에서만 지원돼요.")
    }

    pub fn unprotect(_cipher: &[u8], _entropy: &[u8]) -> anyhow::Result<Vec<u8>> {
        bail!("계정 복호화는 Windows에서만 지원돼요.")
    }
}

pub fn protect(plain: &[u8]) -> anyhow::Result<Vec<u8>> {
    imp::protect(plain, ENTROPY)
}

pub fn unprotect(cipher: &[u8]) -> anyhow::Result<Vec<u8>> {
    imp::unprotect(cipher, ENTROPY)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_current_user_key() {
        let secret = b"refresh-token-value";

        let sealed = protect(secret).unwrap();

        assert_ne!(sealed.as_slice(), secret.as_slice());
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }

    #[test]
    fn an_empty_payload_round_trips() {
        let sealed = protect(b"").unwrap();

        assert_eq!(unprotect(&sealed).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn a_different_entropy_cannot_open_it() {
        let sealed = imp::protect(b"secret", b"entropy-a").unwrap();

        assert!(imp::unprotect(&sealed, b"entropy-b").is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut sealed = protect(b"secret").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;

        assert!(unprotect(&sealed).is_err());
    }
}
