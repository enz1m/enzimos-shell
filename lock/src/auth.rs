//! Password and fingerprint authentication through the system PAM policy.

#![allow(
    unsafe_code,
    reason = "PAM callbacks and libc account lookup expose C pointers."
)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_REFRESH_CRED: c_int = 0x0010;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_AUTHINFO_UNAVAIL: c_int = 9;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}
#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_open_session(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_close_session(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_setcred(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

// Answers every prompt with the typed password; PAM frees the responses,
// so they are allocated with the C allocator.
extern "C" fn conversation(
    count: c_int,
    messages: *mut *const PamMessage,
    out: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if count <= 0 || count > 32 || messages.is_null() || out.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: pam_start receives a live CString as appdata for this synchronous conversation.
    let password = unsafe { &*(data as *const CString) };
    // SAFETY: count is bounded and calloc returns a PAM-compatible, zeroed response array.
    let responses = unsafe { libc::calloc(count as usize, std::mem::size_of::<PamResponse>()) }
        as *mut PamResponse;
    if responses.is_null() {
        return PAM_BUF_ERR;
    }
    for index in 0..count as usize {
        // SAFETY: PAM supplies count message pointers and index is within that array.
        let message = unsafe { *messages.wrapping_add(index) };
        if message.is_null() {
            continue;
        }
        // SAFETY: the non-null message belongs to PAM for the duration of the callback.
        let style = unsafe { (*message).msg_style };
        if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
            // SAFETY: password is NUL-terminated; PAM owns each successfully returned allocation.
            let copy = unsafe { libc::strdup(password.as_ptr()) };
            if copy.is_null() {
                for previous in 0..index {
                    // SAFETY: prior response slots were zeroed or populated by strdup above.
                    let response = unsafe { &*responses.wrapping_add(previous) };
                    // SAFETY: free accepts NULL and each allocation is owned by this callback on failure.
                    unsafe { libc::free(response.resp.cast()) };
                }
                // SAFETY: allocation failed before ownership of the response array passed to PAM.
                unsafe { libc::free(responses.cast()) };
                return PAM_BUF_ERR;
            }
            // SAFETY: index is within the exclusively owned count-element response allocation.
            unsafe { (*responses.wrapping_add(index)).resp = copy };
        }
    }
    // SAFETY: PAM supplies a valid output slot and takes ownership of the response array.
    unsafe { *out = responses };
    PAM_SUCCESS
}

fn account() -> Option<(CString, String)> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let mut storage = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut result = std::ptr::null_mut();
    // SAFETY: record, scratch buffer and result are distinct writable allocations for this call.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            storage.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        return None;
    }
    // SAFETY: successful getpwuid_r initialized storage; its strings remain in the live scratch buffer.
    let entry = unsafe { storage.assume_init_ref() };
    if entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: the non-null pw_name is a NUL-terminated string in the returned passwd record.
    let name = unsafe { CStr::from_ptr(entry.pw_name) }.to_owned();
    let full = if entry.pw_gecos.is_null() {
        String::new()
    } else {
        // SAFETY: the non-null pw_gecos is NUL-terminated and the account scratch buffer remains live.
        let gecos = unsafe { CStr::from_ptr(entry.pw_gecos) }.to_string_lossy();
        gecos.split(',').next().unwrap_or("").trim().to_owned()
    };
    let display = if full.is_empty() {
        name.to_string_lossy().into_owned()
    } else {
        full
    };
    Some((name, display))
}

// Display name for the lock screen: the account's full name, else login.
pub(crate) fn display_name() -> String {
    account().map_or_else(String::new, |(_, display)| display)
}

fn service() -> &'static CStr {
    if std::path::Path::new("/etc/pam.d/enzui-lock").is_file() {
        c"enzui-lock"
    } else {
        c"login"
    }
}

// Checks the current user's password. Blocking; run it off the UI thread.
pub(crate) fn check(password: &str) -> bool {
    // Mock credentials are inert outside an explicitly enabled nested preview.
    #[cfg(feature = "greeter-preview")]
    if std::env::var("ENZUI_SHELL_PLATFORM_PREVIEW").as_deref() == Ok("1")
        && let Ok(expected) = std::env::var("ENZUI_LOCK_TEST_PASSWORD")
    {
        return !expected.is_empty() && expected == password;
    }
    let Some((user, _)) = account() else {
        return false;
    };
    let Ok(secret) = CString::new(password) else {
        return false;
    };
    let conv = PamConv {
        conv: conversation,
        appdata_ptr: &secret as *const CString as *mut c_void,
    };
    let mut handle = std::ptr::null_mut();
    // SAFETY: user, secret, conv and handle storage outlive this synchronous PAM transaction.
    let started = unsafe { pam_start(service().as_ptr(), user.as_ptr(), &conv, &mut handle) };
    if started != PAM_SUCCESS || handle.is_null() {
        return false;
    }
    // This user-run PAM service authenticates only; account expiry and lockout checks belong to
    // login.
    // SAFETY: pam_start succeeded and the conversation data remains live.
    let status = unsafe { pam_authenticate(handle, 0) };
    if status == PAM_SUCCESS {
        // Refreshes credentials such as Kerberos tickets; failure is not a
        // reason to keep the session locked.
        // SAFETY: the authenticated handle remains valid until pam_end.
        unsafe { pam_setcred(handle, PAM_REFRESH_CRED) };
        // oo7 delivers the captured password in its session hook, after authentication.
        // SAFETY: the authenticated handle and conversation remain live for both calls.
        if unsafe { pam_open_session(handle, 0) } == PAM_SUCCESS {
            // SAFETY: close the session opened on this same live PAM handle.
            unsafe { pam_close_session(handle, 0) };
        }
    }
    // SAFETY: this is the sole end call for the successfully started transaction.
    unsafe { pam_end(handle, status) };
    // Best effort: the password bytes do not outlive the check.
    let mut bytes = secret.into_bytes_with_nul();
    bytes.iter_mut().for_each(|b| *b = 0);
    std::hint::black_box(&bytes);
    status == PAM_SUCCESS
}

/// How a fingerprint attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Matched,
    /// The finger did not match, several times.
    Refused,
    /// No reader, no enrolled finger, or no fingerprint service: the lock
    /// shows nothing about fingerprints.
    Unavailable,
}

// Shows fprintd's messages ("Place your finger on the reader") and answers
// any question with nothing: this service never asks for a password.
extern "C" fn fingerprint_conversation(
    count: c_int,
    messages: *mut *const PamMessage,
    out: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if count <= 0 || count > 32 || messages.is_null() || out.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: fingerprint() passes a live callback reference for this synchronous transaction.
    let show = unsafe { &*(data as *const &dyn Fn(String)) };
    // SAFETY: count is bounded and calloc returns a PAM-compatible, zeroed response array.
    let responses = unsafe { libc::calloc(count as usize, std::mem::size_of::<PamResponse>()) }
        as *mut PamResponse;
    if responses.is_null() {
        return PAM_BUF_ERR;
    }
    for index in 0..count as usize {
        // SAFETY: PAM supplies count message pointers and index is within that array.
        let message = unsafe { *messages.wrapping_add(index) };
        if message.is_null() {
            continue;
        }
        // SAFETY: the non-null message belongs to PAM for the duration of the callback.
        let (style, text) = unsafe { ((*message).msg_style, (*message).msg) };
        if (style == PAM_TEXT_INFO || style == PAM_ERROR_MSG) && !text.is_null() {
            // SAFETY: PAM message text is NUL-terminated for the callback's duration.
            let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
            show(text.chars().take(200).collect());
        }
    }
    // SAFETY: PAM supplies a valid output slot and takes ownership of the response array.
    unsafe { *out = responses };
    PAM_SUCCESS
}

/// Whether this system offers fingerprint unlock at all.
pub(crate) fn fingerprint_offered() -> bool {
    std::path::Path::new("/etc/pam.d/enzui-fingerprint").is_file()
        && std::path::Path::new("/usr/lib/security/pam_fprintd.so").is_file()
}

// Waits for the current user's finger through the `enzui-fingerprint` PAM
// service (pam_fprintd only), passing its messages to `show`. Blocking; run
// it beside the password, off the UI thread.
pub(crate) fn fingerprint(show: &dyn Fn(String)) -> Fingerprint {
    let Some((user, _)) = account() else {
        return Fingerprint::Unavailable;
    };
    let data: &dyn Fn(String) = show;
    let conv = PamConv {
        conv: fingerprint_conversation,
        appdata_ptr: &data as *const &dyn Fn(String) as *mut c_void,
    };
    let mut handle = std::ptr::null_mut();
    // SAFETY: user, the callback and handle storage outlive this synchronous PAM transaction.
    let started = unsafe {
        pam_start(
            c"enzui-fingerprint".as_ptr(),
            user.as_ptr(),
            &conv,
            &mut handle,
        )
    };
    if started != PAM_SUCCESS || handle.is_null() {
        return Fingerprint::Unavailable;
    }
    // SAFETY: pam_start succeeded and the conversation data remains live.
    let status = unsafe { pam_authenticate(handle, 0) };
    // SAFETY: this is the sole end call for the successfully started transaction.
    unsafe { pam_end(handle, status) };
    match status {
        PAM_SUCCESS => Fingerprint::Matched,
        PAM_AUTHINFO_UNAVAIL => Fingerprint::Unavailable,
        _ => Fingerprint::Refused,
    }
}
