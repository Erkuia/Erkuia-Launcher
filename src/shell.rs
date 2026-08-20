#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use anyhow::bail;

    const SW_SHOWNORMAL: i32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const SHELL_EXECUTE_SUCCESS_THRESHOLD: isize = 32;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut c_void;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn open(target: &str) -> anyhow::Result<()> {
        unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };

        let operation = wide("open");
        let file = wide(target);

        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };

        if (result as isize) <= SHELL_EXECUTE_SUCCESS_THRESHOLD {
            bail!(
                "열지 못했어요: {target} (오류 {})",
                std::io::Error::last_os_error()
            );
        }

        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::bail;

    pub fn open(target: &str) -> anyhow::Result<()> {
        bail!("이 플랫폼에서는 열 수 없어요: {target}")
    }
}

pub fn open(target: &str) -> anyhow::Result<()> {
    log::info!("셸로 열기: {target}");

    imp::open(target)
}
