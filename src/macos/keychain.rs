//! Bot credentials stay in Keychain, outside preferences and process arguments.
use anyhow::{Result, bail};
use std::{
    ffi::{CStr, CString, c_void},
    ptr,
};
type Ref = *const c_void;
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
    fn SecItemAdd(attributes: Ref, result: *mut Ref) -> i32;
    fn SecItemUpdate(query: Ref, attributes: Ref) -> i32;
    fn SecItemDelete(query: Ref) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryCreateMutable(allocator: Ref, capacity: isize, keys: Ref, values: Ref) -> Ref;
    fn CFDictionarySetValue(dictionary: Ref, key: Ref, value: Ref);
    fn CFStringCreateWithCString(allocator: Ref, text: *const i8, encoding: u32) -> Ref;
    fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFDataGetLength(data: Ref) -> isize;
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFRelease(value: Ref);
}
// Framework constants are resolved into a short-lived context, not cached globally.
struct Symbols {
    class: Ref,
    password: Ref,
    service: Ref,
    account: Ref,
    data: Ref,
    return_data: Ref,
    boolean_true: Ref,
}
fn symbol(name: &CStr) -> Result<Ref> {
    // SAFETY: These names are public framework data symbols of type CFTypeRef.
    // Both frameworks are linked below and remain loaded for the process lifetime.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    anyhow::ensure!(!address.is_null(), "Keychain API is unavailable");
    let value = unsafe { *address.cast::<Ref>() };
    anyhow::ensure!(!value.is_null(), "Keychain API constant is unavailable");
    Ok(value)
}
impl Symbols {
    fn load() -> Result<Self> {
        Ok(Self {
            class: symbol(c"kSecClass")?,
            password: symbol(c"kSecClassGenericPassword")?,
            service: symbol(c"kSecAttrService")?,
            account: symbol(c"kSecAttrAccount")?,
            data: symbol(c"kSecValueData")?,
            return_data: symbol(c"kSecReturnData")?,
            boolean_true: symbol(c"kCFBooleanTrue")?,
        })
    }
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
    let text = CString::new(text).expect("keychain identity");
    Owned(unsafe { CFStringCreateWithCString(ptr::null(), text.as_ptr(), 0x08000100) })
}
fn dictionary() -> Owned {
    Owned(unsafe { CFDictionaryCreateMutable(ptr::null(), 0, ptr::null(), ptr::null()) })
}
fn query() -> Result<(Owned, Owned, Owned, Symbols)> {
    let keys = Symbols::load()?;
    let query = dictionary();
    let service = string("local.codex.agent-monitor.notify.telegram");
    let account = string("bot-token");
    // The dictionary borrows its values. These owned strings live through each call.
    unsafe {
        CFDictionarySetValue(query.0, keys.class, keys.password);
        CFDictionarySetValue(query.0, keys.service, service.0);
        CFDictionarySetValue(query.0, keys.account, account.0);
    }
    Ok((query, service, account, keys))
}
pub fn load() -> Result<Option<String>> {
    let (query, _service, _account, keys) = query()?;
    let mut result = ptr::null();
    let status = unsafe {
        CFDictionarySetValue(query.0, keys.return_data, keys.boolean_true);
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
    let (query, _service, _account, keys) = query()?;
    let data = Owned(unsafe { CFDataCreate(ptr::null(), token.as_ptr(), token.len() as isize) });
    let attributes = dictionary();
    let status = unsafe {
        CFDictionarySetValue(attributes.0, keys.data, data.0);
        let status = SecItemUpdate(query.0, attributes.0);
        if status == -25300 {
            CFDictionarySetValue(query.0, keys.data, data.0);
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
    let (query, _service, _account, _keys) = query()?;
    let status = unsafe { SecItemDelete(query.0) };
    if status != 0 && status != -25300 {
        bail!("Could not remove Telegram token from Keychain ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn framework_constants_are_resolved_without_loading_credentials() {
        let symbols = super::Symbols::load().unwrap();
        assert!(!symbols.class.is_null());
        assert!(!symbols.boolean_true.is_null());
    }
}
