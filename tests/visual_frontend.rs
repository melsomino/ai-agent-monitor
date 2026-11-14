//! Runs the actual AppKit menu renderer on the main thread without showing UI,
//! starting actors, accessing credentials, or capturing the user's screen.
#![allow(dead_code, unused_imports)]
#[cfg(target_os = "macos")]
#[path = "../src/actors/mod.rs"]
mod actors;
#[cfg(target_os = "macos")]
#[path = "../src/agents/mod.rs"]
mod agents;
#[cfg(target_os = "macos")]
#[path = "../src/app/mod.rs"]
mod app;
#[cfg(target_os = "macos")]
#[path = "../src/domain/mod.rs"]
mod domain;
#[cfg(target_os = "macos")]
#[path = "../src/macos/mod.rs"]
mod macos;
#[cfg(target_os = "macos")]
#[path = "../src/platform.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "../src/robot_motion.rs"]
mod robot_motion;
#[cfg(target_os = "macos")]
#[path = "../src/web/server/mod.rs"]
mod web;

#[cfg(target_os = "macos")]
fn main() {
    let mtm = objc2::MainThreadMarker::new().expect("AppKit must run on the main thread");
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Prohibited);
    macos::menu::visual_test::verify(mtm);
}
#[cfg(not(target_os = "macos"))]
fn main() {}
