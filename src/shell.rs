#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use anyhow::bail;

    const SW_SHOWNORMAL: i32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const SHELL_EXECUTE_SUCCESS_THRESHOLD: isize = 32;

    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
    const INFINITE: u32 = 0xFFFF_FFFF;
    const ERROR_CANCELLED: i32 = 1223;

    #[repr(C)]
    struct ShellExecuteInfoW {
        size: u32,
        mask: u32,
        hwnd: *mut c_void,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
        instance: *mut c_void,
        id_list: *mut c_void,
        class: *const u16,
        key_class: *mut c_void,
        hot_key: u32,
        icon_or_monitor: *mut c_void,
        process: *mut c_void,
    }

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

        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
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

    /// Runs `exe` through the `runas` verb, which is what raises the UAC prompt,
    /// and blocks until it finishes.
    ///
    /// Waiting is the point. The caller has to know whether the elevated half
    /// succeeded before it acts on the result, and it must not hand the new
    /// process its own elevation — so the elevated child does the privileged
    /// work and nothing else, and the unprivileged parent carries on afterwards.
    pub fn run_as_admin_and_wait(exe: &str, parameters: &str) -> anyhow::Result<u32> {
        unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };

        let verb = wide("runas");
        let file = wide(exe);
        let arguments = wide(parameters);

        let mut info = ShellExecuteInfoW {
            size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
            mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            hwnd: std::ptr::null_mut(),
            verb: verb.as_ptr(),
            file: file.as_ptr(),
            parameters: arguments.as_ptr(),
            directory: std::ptr::null(),
            show: SW_SHOWNORMAL,
            instance: std::ptr::null_mut(),
            id_list: std::ptr::null_mut(),
            class: std::ptr::null(),
            key_class: std::ptr::null_mut(),
            hot_key: 0,
            icon_or_monitor: std::ptr::null_mut(),
            process: std::ptr::null_mut(),
        };

        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let error = std::io::Error::last_os_error();

            if error.raw_os_error() == Some(ERROR_CANCELLED) {
                bail!("관리자 권한 요청이 취소됐어요.");
            }

            bail!("관리자 권한으로 실행하지 못했어요: {error}");
        }

        if info.process.is_null() {
            bail!("관리자 권한 프로세스를 추적하지 못했어요.");
        }

        unsafe { WaitForSingleObject(info.process, INFINITE) };

        let mut code: u32 = 0;
        let read = unsafe { GetExitCodeProcess(info.process, &mut code) };
        unsafe { CloseHandle(info.process) };

        if read == 0 {
            bail!("관리자 권한 프로세스의 결과를 읽지 못했어요.");
        }

        Ok(code)
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::bail;

    pub fn open(target: &str) -> anyhow::Result<()> {
        bail!("이 플랫폼에서는 열 수 없어요: {target}")
    }

    pub fn run_as_admin_and_wait(exe: &str, _parameters: &str) -> anyhow::Result<u32> {
        bail!("이 플랫폼에서는 관리자 권한으로 실행할 수 없어요: {exe}")
    }
}

pub fn open(target: &str) -> anyhow::Result<()> {
    log::info!("셸로 열기: {target}");

    imp::open(target)
}

pub fn run_as_admin_and_wait(exe: &str, parameters: &str) -> anyhow::Result<u32> {
    log::info!("관리자 권한으로 실행: {exe} {parameters}");

    imp::run_as_admin_and_wait(exe, parameters)
}
