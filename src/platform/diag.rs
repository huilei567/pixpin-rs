//! Last-resort crash reporting.
//!
//! GPUI's event loop swallows nothing, so a silent process death means a real
//! access violation or an abort. This installs a panic hook plus an unhandled
//! exception filter so those deaths leave a breadcrumb on disk instead of
//! vanishing without a trace.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::{
    SetUnhandledExceptionFilter, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

const LOG: &str = "pixpin-crash.log";

static LOCK: Mutex<()> = Mutex::new(());

fn append(line: &str) {
    let _guard = LOCK.lock();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
    {
        let _ = writeln!(f, "{line}");
        let _ = f.flush();
    }
    eprintln!("[pixpin-rs] {line}");
}

fn module_of(addr: usize) -> String {
    unsafe {
        let mut hmod = HMODULE::default();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(addr as *const u16),
            &mut hmod,
        )
        .is_err()
        {
            return "<unknown>".into();
        }
        let mut buf = [0u16; 260];
        let n = GetModuleFileNameW(Some(hmod), &mut buf);
        let name = String::from_utf16_lossy(&buf[..n as usize]);
        let base = hmod.0 as usize;
        format!("{name}+0x{:x}", addr.wrapping_sub(base))
    }
}

unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
    if !info.is_null() {
        let rec = (*info).ExceptionRecord;
        if !rec.is_null() {
            let code = (*rec).ExceptionCode.0 as u32;
            let addr = (*rec).ExceptionAddress as usize;
            append(&format!(
                "UNHANDLED code=0x{code:08X} at 0x{addr:x} ({} )",
                module_of(addr)
            ));
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// Call as early as possible in `main`.
pub fn install() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |p| {
        append(&format!("PANIC {p}"));
        prev(p);
    }));
    unsafe {
        SetUnhandledExceptionFilter(Some(filter));
    }
}

// ---------------------------------------------------------------------------
// Opt-in trace log
// ---------------------------------------------------------------------------
//
// The GUI build frees its console (see `hide_own_console`), so `eprintln!` goes
// nowhere on a normal double-click launch. When `--trace` or `PIXPIN_TRACE=1`
// is set we mirror the interesting events into `pixpin-trace.log` next to the
// working directory, which is the only way to see what the hotkey path did on a
// machine we cannot attach a debugger to.

static TRACE: AtomicBool = AtomicBool::new(false);

const TRACE_LOG: &str = "pixpin-trace.log";

pub fn enable_trace() {
    TRACE.store(true, Ordering::Relaxed);
}

/// Monotonic-ish millisecond stamp, good enough to order events.
fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn trace(msg: &str) {
    if !TRACE.load(Ordering::Relaxed) {
        return;
    }
    let _guard = LOCK.lock();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(TRACE_LOG)
    {
        let _ = writeln!(f, "{} {}", stamp(), msg);
        let _ = f.flush();
    }
}
