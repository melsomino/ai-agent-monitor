//! Bot credentials stay in Keychain, outside preferences and process arguments.
use anyhow::{Result, bail};
use std::{
    ffi::{CString, c_void},
    ptr,
};
type Ref = *const c_void;
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: Ref;
    static kSecClassGenericPassword: Ref;
    static kSecAttrService: Ref;
    static kSecAttrAccount: Ref;
    static kSecValueData: Ref;
    static kSecReturnData: Ref;
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
    fn SecItemAdd(attributes: Ref, result: *mut Ref) -> i32;
    fn SecItemUpdate(query: Ref, attributes: Ref) -> i32;
    fn SecItemDelete(query: Ref) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: Ref;
    fn CFDictionaryCreateMutable(allocator: Ref, capacity: isize, keys: Ref, values: Ref) -> Ref;
    fn CFDictionarySetValue(dictionary: Ref, key: Ref, value: Ref);
    fn CFStringCreateWithCString(allocator: Ref, text: *const i8, encoding: u32) -> Ref;
    fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFDataGetLength(data: Ref) -> isize;
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFRelease(value: Ref);
}
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}
fn string(text: &str) -> Owned {
    let text = CString::new(text).expect("static keychain identity");
    Owned(unsafe { CFStringCreateWithCString(ptr::null(), text.as_ptr(), 0x08000100) })
}
fn dictionary() -> Owned {
    Owned(unsafe { CFDictionaryCreateMutable(ptr::null(), 0, ptr::null(), ptr::null()) })
}
fn query() -> (Owned, Owned, Owned) {
    let query = dictionary();
    let service = string("local.codex.agent-monitor.notify.telegram");
    let account = string("bot-token");
    // The dictionary borrows its values. These owned strings live through each call.
    unsafe {
        CFDictionarySetValue(query.0, kSecClass, kSecClassGenericPassword);
        CFDictionarySetValue(query.0, kSecAttrService, service.0);
        CFDictionarySetValue(query.0, kSecAttrAccount, account.0);
    }
    (query, service, account)
}
pub fn load() -> Result<Option<String>> {
    let (query, _service, _account) = query();
    let mut result = ptr::null();
    let status = unsafe {
        CFDictionarySetValue(query.0, kSecReturnData, kCFBooleanTrue);
        SecItemCopyMatching(query.0, &mut result)
    };
    if status == -25300 {
        return Ok(None);
    }
    if status != 0 {
        bail!("Could not read Telegram token from Keychain ({status})");
    }
    let data = Owned(result);
    let length = unsafe { CFDataGetLength(data.0) };
    anyhow::ensure!(
        length > 0 && length <= 256,
        "Invalid Telegram credential in Keychain"
    );
    let bytes = unsafe { std::slice::from_raw_parts(CFDataGetBytePtr(data.0), length as usize) };
    Ok(Some(String::from_utf8(bytes.to_vec())?))
}
pub fn save(token: &str) -> Result<()> {
    let (query, _service, _account) = query();
    let data = Owned(unsafe { CFDataCreate(ptr::null(), token.as_ptr(), token.len() as isize) });
    let attributes = dictionary();
    let status = unsafe {
        CFDictionarySetValue(attributes.0, kSecValueData, data.0);
        let status = SecItemUpdate(query.0, attributes.0);
        if status == -25300 {
            CFDictionarySetValue(query.0, kSecValueData, data.0);
            SecItemAdd(query.0, ptr::null_mut())
        } else {
            status
        }
    };
    if status != 0 {
        bail!("Could not save Telegram token in Keychain ({status})");
    }
    Ok(())
}
pub fn delete() -> Result<()> {
    let (query, _service, _account) = query();
    let status = unsafe { SecItemDelete(query.0) };
    if status != 0 && status != -25300 {
        bail!("Could not remove Telegram token from Keychain ({status})");
    }
    Ok(())
}
