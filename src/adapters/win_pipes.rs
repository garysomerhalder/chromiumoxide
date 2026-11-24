//! Windows: start a browser with pipes on its C runtime's file descriptors
//! 3 and 4, the way Node (and so Playwright) does: the handles travel in
//! `STARTUPINFO.lpReserved2`, the block the MSVC runtime reads its inherited
//! descriptors from. Only those handles (and stderr's) are inherited
//! (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), so the browser sees end of file
//! when this process goes away and quits, like on Unix.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::domain::error::{Error, Result};
use crate::domain::launch::windows_command_line;
use crate::ports::BrowserProcess;

/// An owned Windows handle.
struct Owned(HANDLE);

// SAFETY: a process or pipe handle may be used and closed from any thread.
unsafe impl Send for Owned {}
// SAFETY: as above; the handle is only read.
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

impl Owned {
    /// Hands the handle over (it is not closed here).
    fn into_raw(self) -> HANDLE {
        let h = self.0;
        std::mem::forget(self);
        h
    }
}

fn last_error(what: &str) -> Error {
    Error::Launch(format!("{what}: {}", std::io::Error::last_os_error()))
}

/// A pipe whose `child` end is inheritable and whose other end is not.
fn pipe(child_reads: bool) -> Result<(Owned, Owned)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: valid out pointers and attributes.
    if unsafe { CreatePipe(&mut read, &mut write, &sa, 0) } == 0 {
        return Err(last_error("cannot create a pipe"));
    }
    let (read, write) = (Owned(read), Owned(write));
    let (child, parent) = if child_reads {
        (read, write)
    } else {
        (write, read)
    };
    // SAFETY: a valid handle we own.
    if unsafe { SetHandleInformation(parent.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(last_error("cannot set up a pipe"));
    }
    Ok((child, parent))
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// This process's environment plus `env`, as a sorted
/// `CREATE_UNICODE_ENVIRONMENT` block.
fn environment_block(env: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    for (k, v) in env {
        vars.retain(|(name, _)| !name.eq_ignore_ascii_case(k));
        vars.push((k.clone(), v.clone()));
    }
    vars.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(OsStr::new(&format!("{k}={v}")).encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

/// The MSVC runtime's inherited-descriptor block: a count, one flag byte
/// per descriptor, then the handles (unaligned). Descriptors 0-2 are left
/// to the standard handles; 3 and 4 are the pipes (`FOPEN | FPIPE`).
fn crt_block(fd3: HANDLE, fd4: HANDLE) -> Vec<u8> {
    const FOPEN_FPIPE: u8 = 0x01 | 0x08;
    let handles = [
        INVALID_HANDLE_VALUE,
        INVALID_HANDLE_VALUE,
        INVALID_HANDLE_VALUE,
        fd3,
        fd4,
    ];
    let mut block = (handles.len() as u32).to_le_bytes().to_vec();
    block.extend([0, 0, 0, FOPEN_FPIPE, FOPEN_FPIPE]);
    for h in handles {
        block.extend((h as usize).to_le_bytes());
    }
    block
}

/// A browser started with [`spawn`].
pub struct WinChild {
    process: Arc<Owned>,
    pid: u32,
}

impl WinChild {
    fn wait(process: &Owned, timeout: Duration) -> bool {
        // SAFETY: a valid process handle.
        unsafe { WaitForSingleObject(process.0, timeout.as_millis() as u32) == WAIT_OBJECT_0 }
    }
}

impl Drop for WinChild {
    fn drop(&mut self) {
        if !Self::wait(&self.process, Duration::ZERO) {
            // SAFETY: a valid process handle.
            unsafe { TerminateProcess(self.process.0, 1) };
        }
    }
}

impl BrowserProcess for WinChild {
    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }

    fn shutdown(&self) -> BoxFuture<'static, ()> {
        let process = self.process.clone();
        async move {
            let _ = tokio::task::spawn_blocking(move || {
                if !WinChild::wait(&process, Duration::from_secs(5)) {
                    // SAFETY: a valid process handle.
                    unsafe { TerminateProcess(process.0, 1) };
                }
            })
            .await;
        }
        .boxed()
    }
}

/// A started browser: the process, our ends of its fd 3 (we write) and
/// fd 4 (we read), and its stderr.
pub struct Spawned {
    pub child: WinChild,
    pub to_child: std::fs::File,
    pub from_child: std::fs::File,
    pub stderr: std::fs::File,
}

/// Starts `program` with `args` and `env` added, its fd 3 and 4 wired to
/// pipes.
pub fn spawn(program: &Path, args: &[String], env: &[(String, String)]) -> Result<Spawned> {
    let (child_read, parent_write) = pipe(true)?;
    let (child_write, parent_read) = pipe(false)?;
    let (child_err, parent_err) = pipe(false)?;
    let mut crt = crt_block(child_read.0, child_write.0);
    let mut inherited: [HANDLE; 3] = [child_read.0, child_write.0, child_err.0];

    let mut size = 0usize;
    // SAFETY: the documented size query (a null list).
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    let mut list_buf = vec![0u8; size];
    let list = list_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    // SAFETY: a buffer of the size Windows asked for.
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(last_error("cannot create the attribute list"));
    }
    struct ListGuard(LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for ListGuard {
        fn drop(&mut self) {
            // SAFETY: initialised above, deleted once.
            unsafe { DeleteProcThreadAttributeList(self.0) };
        }
    }
    let _guard = ListGuard(list);
    // SAFETY: `inherited` outlives the CreateProcessW call below.
    if unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited.as_mut_ptr().cast(),
            std::mem::size_of_val(&inherited),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(last_error("cannot list the inherited handles"));
    }

    let mut info: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    info.StartupInfo.hStdInput = std::ptr::null_mut();
    info.StartupInfo.hStdOutput = std::ptr::null_mut();
    info.StartupInfo.hStdError = child_err.0;
    info.StartupInfo.cbReserved2 = crt.len() as u16;
    info.StartupInfo.lpReserved2 = crt.as_mut_ptr();
    info.lpAttributeList = list;

    let mut command_line = wide(OsStr::new(&windows_command_line(program, args)));
    let mut environment = environment_block(env);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer is valid for the call; the strings are
    // NUL-terminated and the command line buffer is mutable.
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
            environment.as_mut_ptr().cast(),
            std::ptr::null(),
            &info.StartupInfo,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(Error::Launch(format!(
            "{}: {}",
            program.display(),
            std::io::Error::last_os_error()
        )));
    }
    drop(Owned(pi.hThread));
    // The child has its copies now.
    drop((child_read, child_write, child_err));
    // SAFETY: each handle is owned and handed to exactly one File.
    let file = |h: Owned| unsafe { std::fs::File::from_raw_handle(h.into_raw() as RawHandle) };
    Ok(Spawned {
        child: WinChild {
            process: Arc::new(Owned(pi.hProcess)),
            pid: pi.dwProcessId,
        },
        to_child: file(parent_write),
        from_child: file(parent_read),
        stderr: file(parent_err),
    })
}
