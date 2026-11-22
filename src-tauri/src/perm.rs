//! 屏幕录制权限（与 desktop 壳同一套机制，见 desktop/src-tauri/src/perm.rs）。
//!
//! macOS 把「能不能抓屏」算在**发起进程**的 TCC 身份上，而不是真正干活的那个命令：
//! screencapture 是客户端 fork 出来的子进程，权限记在它爹头上。所以客户端自己必须能被
//! 授权，也必须能被系统问过一次——否则系统设置里根本不会出现这一项，人想勾也找不到。
//!
//! 查/要权限的两个函数在 CoreGraphics 里，10.15 才有。这里用 dlsym 取符号而不是
//! 直接 extern 链接：老系统上缺符号会让进程一启动就挂，而缺符号本来也只是「查不了」。

#![allow(dead_code)]

use std::ffi::c_void;

#[cfg(target_os = "macos")]
const FRAMEWORK: &[u8] = b"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics\0";

#[cfg(target_os = "macos")]
unsafe fn lookup(name: &[u8]) -> Option<*mut c_void> {
    let handle = libc::dlopen(FRAMEWORK.as_ptr().cast(), libc::RTLD_LAZY);
    if handle.is_null() {
        return None;
    }
    let symbol = libc::dlsym(handle, name.as_ptr().cast());
    if symbol.is_null() {
        None
    } else {
        Some(symbol)
    }
}

/// 已经授过权了吗。查不到接口（10.15 之前）就当有——不给人添堵。
pub fn granted() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        match lookup(b"CGPreflightScreenCaptureAccess\0") {
            Some(symbol) => {
                let probe: unsafe extern "C" fn() -> bool = std::mem::transmute(symbol);
                probe()
            }
            None => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// 弹一次系统授权框，把本应用登记进「屏幕录制」面板。
///
/// 返回 true 表示当场就拿到了（用户之前已经勾过、只是进程还没重启）。
/// 返回 false 是常态：系统会把人领到设置面板，勾完必须重开进程才生效。
/// 注意 preflight 在「进程起来之后才授权」时会一直返回 false，所以判断要信这个的返回值。
pub fn request() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        match lookup(b"CGRequestScreenCaptureAccess\0") {
            Some(symbol) => {
                let ask: unsafe extern "C" fn() -> bool = std::mem::transmute(symbol);
                ask()
            }
            None => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    true
}
