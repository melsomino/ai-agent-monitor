//! Global shortcuts via the public Carbon hot-key API (no keyboard event tap).
use anyhow::{Result, bail};
use objc2::MainThreadMarker;
use objc2_app_kit::NSEventModifierFlags;
use serde_json::{Value, json};
use std::{ffi::c_void, marker::PhantomData, ptr, rc::Rc};

const COMMAND: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;
const MODIFIERS: u32 = COMMAND | SHIFT | OPTION | CONTROL;
const SIGNATURE: u32 = u32::from_be_bytes(*b"CDXN");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key_code: u32,
    pub modifiers: u32,
    pub key_name: String,
}

impl Shortcut {
    pub fn from_event(
        key_code: u16,
        flags: NSEventModifierFlags,
        characters: &str,
    ) -> Option<Self> {
        let mut modifiers = 0;
        for (flag, value) in [
            (NSEventModifierFlags::Command, COMMAND),
            (NSEventModifierFlags::Shift, SHIFT),
            (NSEventModifierFlags::Option, OPTION),
            (NSEventModifierFlags::Control, CONTROL),
        ] {
            if flags.contains(flag) {
                modifiers |= value;
            }
        }
        let shortcut = Self {
            key_code: key_code.into(),
            modifiers,
            key_name: key_name(key_code, characters),
        };
        shortcut.valid().then_some(shortcut)
    }

    fn valid(&self) -> bool {
        self.key_code <= 127
            && self.modifiers & !MODIFIERS == 0
            && self.modifiers & (COMMAND | OPTION | CONTROL) != 0
            && !self.key_name.is_empty()
            && self.key_name.len() <= 64
            && !self.key_name.chars().any(char::is_control)
    }

    pub fn label(&self) -> String {
        let mut label = String::new();
        for (mask, symbol) in [(CONTROL, "⌃"), (OPTION, "⌥"), (SHIFT, "⇧"), (COMMAND, "⌘")]
        {
            if self.modifiers & mask != 0 {
                label.push_str(symbol);
            }
        }
        label.push_str(&self.key_name);
        label
    }

    pub fn cocoa_modifiers(&self) -> NSEventModifierFlags {
        let mut flags = NSEventModifierFlags::empty();
        for (mask, flag) in [
            (COMMAND, NSEventModifierFlags::Command),
            (SHIFT, NSEventModifierFlags::Shift),
            (OPTION, NSEventModifierFlags::Option),
            (CONTROL, NSEventModifierFlags::Control),
        ] {
            if self.modifiers & mask != 0 {
                flags |= flag;
            }
        }
        flags
    }

    pub fn menu_key_equivalent(&self) -> String {
        let character = match self.key_code {
            36 => Some('\r'),
            76 => Some('\u{3}'),
            48 => Some('\t'),
            49 => Some(' '),
            51 => Some('\u{7f}'),
            53 => Some('\u{1b}'),
            115 => Some('\u{f729}'),
            116 => Some('\u{f72c}'),
            117 => Some('\u{f728}'),
            119 => Some('\u{f72b}'),
            121 => Some('\u{f72d}'),
            123 => Some('\u{f702}'),
            124 => Some('\u{f703}'),
            125 => Some('\u{f701}'),
            126 => Some('\u{f700}'),
            _ => self
                .key_name
                .strip_prefix('F')
                .and_then(|number| number.parse::<u32>().ok())
                .filter(|number| (1..=20).contains(number))
                .and_then(|number| char::from_u32(0xf703 + number)),
        };
        character
            .map(|character| character.to_string())
            .unwrap_or_else(|| self.key_name.to_lowercase())
    }

    pub fn encode(&self) -> String {
        json!({"keyCode": self.key_code, "modifiers": self.modifiers, "keyName": self.key_name})
            .to_string()
    }

    pub fn decode(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let shortcut = Self {
            key_code: value["keyCode"].as_u64()?.try_into().ok()?,
            modifiers: value["modifiers"].as_u64()?.try_into().ok()?,
            key_name: value["keyName"].as_str()?.into(),
        };
        shortcut.valid().then_some(shortcut)
    }
}

fn key_name(code: u16, characters: &str) -> String {
    match code {
        36 | 76 => "↩".into(),
        48 => "⇥".into(),
        49 => "Space".into(),
        51 => "⌫".into(),
        53 => "⎋".into(),
        115 => "↖".into(),
        116 => "⇞".into(),
        117 => "⌦".into(),
        119 => "↘".into(),
        121 => "⇟".into(),
        123 => "←".into(),
        124 => "→".into(),
        125 => "↓".into(),
        126 => "↑".into(),
        _ => {
            let function = [
                (122, 1),
                (120, 2),
                (99, 3),
                (118, 4),
                (96, 5),
                (97, 6),
                (98, 7),
                (100, 8),
                (101, 9),
                (109, 10),
                (103, 11),
                (111, 12),
                (105, 13),
                (107, 14),
                (113, 15),
                (106, 16),
                (64, 17),
                (79, 18),
                (80, 19),
                (90, 20),
            ];
            if let Some((_, number)) = function.iter().find(|(key, _)| *key == code) {
                return format!("F{number}");
            }
            let name = characters.trim().to_uppercase();
            if name.is_empty() || name.chars().any(char::is_control) {
                format!("Key {code}")
            } else {
                name
            }
        }
    }
}

#[repr(C)]
struct EventType {
    class: u32,
    kind: u32,
}
#[repr(C)]
#[derive(Default)]
struct HotKeyId {
    signature: u32,
    id: u32,
}
type Handler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetEventDispatcherTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        count: usize,
        types: *const EventType,
        data: *mut c_void,
        result: *mut *mut c_void,
    ) -> i32;
    fn RemoveEventHandler(handler: *mut c_void) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: HotKeyId,
        target: *mut c_void,
        options: u32,
        result: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(key: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        kind: u32,
        actual_kind: *mut u32,
        size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
}

unsafe extern "C" fn pressed(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> i32 {
    let mut id = HotKeyId::default();
    // SAFETY: Carbon supplies a live keyboard event. The direct parameter is
    // copied into a correctly sized, repr(C) EventHotKeyID from the SDK header.
    let status = unsafe {
        GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            ptr::null_mut(),
            size_of::<HotKeyId>(),
            ptr::null_mut(),
            (&mut id as *mut HotKeyId).cast(),
        )
    };
    if status != 0 || id.signature != SIGNATURE || id.id != 1 {
        return -9874;
    }
    if MainThreadMarker::new().is_some() {
        crate::menu::toggle_from_hotkey();
    } else {
        dispatch2::DispatchQueue::main().exec_async(crate::menu::toggle_from_hotkey);
    }
    0
}

pub struct Registration {
    key: *mut c_void,
    handler: *mut c_void,
    _main_thread: PhantomData<Rc<()>>,
}

impl Registration {
    pub fn new(shortcut: &Shortcut, _mtm: MainThreadMarker) -> Result<Self> {
        anyhow::ensure!(shortcut.valid(), "Use ⌘, ⌥ or ⌃ with a key.");
        let mut registration = Self {
            key: ptr::null_mut(),
            handler: ptr::null_mut(),
            _main_thread: PhantomData,
        };
        // SAFETY: SDK signatures match the FFI declarations. Registration and
        // teardown are main-thread-only. The callback has no borrowed user data.
        unsafe {
            let target = GetEventDispatcherTarget();
            let event_type = EventType {
                class: u32::from_be_bytes(*b"keyb"),
                kind: 5,
            };
            let status = InstallEventHandler(
                target,
                pressed,
                1,
                &event_type,
                ptr::null_mut(),
                &mut registration.handler,
            );
            anyhow::ensure!(
                status == 0,
                "Could not install shortcut handler ({status})."
            );
            let status = RegisterEventHotKey(
                shortcut.key_code,
                shortcut.modifiers,
                HotKeyId {
                    signature: SIGNATURE,
                    id: 1,
                },
                target,
                1,
                &mut registration.key,
            );
            if status == -9878 {
                bail!("This shortcut is already in use. Try another.");
            }
            anyhow::ensure!(status == 0, "Could not register this shortcut ({status}).");
        }
        Ok(registration)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: These handles are owned, were created on this thread, and
        // cannot be sent to another thread. Null handles were never installed.
        unsafe {
            if !self.key.is_null() {
                UnregisterEventHotKey(self.key);
            }
            if !self.handler.is_null() {
                RemoveEventHandler(self.handler);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_preserves_physical_key_and_translates_cocoa_modifier_bits() {
        let key = Shortcut::from_event(
            40,
            NSEventModifierFlags::Command | NSEventModifierFlags::Option,
            "k",
        )
        .unwrap();
        assert_eq!(key.modifiers, COMMAND | OPTION);
        assert_eq!(key.label(), "⌥⌘K");
        assert_eq!(key.menu_key_equivalent(), "k");
        assert_eq!(
            key.cocoa_modifiers(),
            NSEventModifierFlags::Command | NSEventModifierFlags::Option
        );
        assert_eq!(Shortcut::decode(&key.encode()), Some(key));
        assert_eq!(
            Shortcut::from_event(49, NSEventModifierFlags::Control, " ")
                .unwrap()
                .label(),
            "⌃Space"
        );
        assert_eq!(
            Shortcut::from_event(123, NSEventModifierFlags::Command, "")
                .unwrap()
                .label(),
            "⌘←"
        );
    }
    #[test]
    fn invalid_preferences_and_plain_typing_cannot_claim_a_global_shortcut() {
        assert!(Shortcut::from_event(40, NSEventModifierFlags::Shift, "K").is_none());
        for value in [
            "null",
            "{}",
            r#"{"keyCode":999,"modifiers":256,"keyName":"K"}"#,
            r#"{"keyCode":40,"modifiers":0,"keyName":"K"}"#,
            r#"{"keyCode":40,"modifiers":1,"keyName":"K"}"#,
            r#"{"keyCode":40,"modifiers":256,"keyName":"\n"}"#,
        ] {
            assert!(Shortcut::decode(value).is_none());
        }
    }

    #[test]
    fn native_menu_equivalents_use_event_characters_instead_of_display_symbols() {
        for (code, name, equivalent) in [
            (44, "/", "/"),
            (49, " ", " "),
            (123, "", "\u{f702}"),
            (36, "", "\r"),
            (51, "", "\u{7f}"),
            (117, "", "\u{f728}"),
            (122, "", "\u{f704}"),
            (90, "", "\u{f717}"),
        ] {
            let shortcut = Shortcut::from_event(
                code,
                NSEventModifierFlags::Control | NSEventModifierFlags::Shift,
                name,
            )
            .unwrap();
            assert_eq!(shortcut.menu_key_equivalent(), equivalent);
            assert_eq!(
                shortcut.cocoa_modifiers(),
                NSEventModifierFlags::Control | NSEventModifierFlags::Shift
            );
        }
    }
}
