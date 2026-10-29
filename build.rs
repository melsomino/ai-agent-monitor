use std::{
    env,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
    path::PathBuf,
};

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    // This intentionally absent input makes even a repeated Cargo build receive
    // a new number. Packaging an existing binary preserves its compiled number.
    println!(
        "cargo:rerun-if-changed={}/next-build",
        env::var("OUT_DIR").unwrap()
    );
    println!("cargo:rerun-if-changed=build.rs");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".build-number.lock"))
        .expect("open build-number lock");
    // SAFETY: Hold an exclusive, process-shared lock on this owned descriptor.
    assert_eq!(
        unsafe { flock(lock.as_raw_fd(), 2) },
        0,
        "lock build number"
    );
    let mut counter = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("build-number.txt"))
        .expect("open build-number.txt");
    let mut previous = String::new();
    counter
        .read_to_string(&mut previous)
        .expect("read build number");
    let number = previous
        .trim()
        .parse::<u64>()
        .expect("invalid build number")
        .checked_add(1)
        .expect("build number exhausted");
    let data = format!("{number}\n");
    counter.seek(SeekFrom::Start(0)).unwrap();
    counter
        .write_all(data.as_bytes())
        .expect("write build number");
    counter.set_len(data.len() as u64).unwrap();
    counter.sync_data().unwrap();
    println!("cargo:rustc-env=AI_AGENT_MONITOR_BUILD={number}");
    println!(
        "cargo:rustc-env=AI_AGENT_MONITOR_VERSION={}+{number}",
        env::var("CARGO_PKG_VERSION").unwrap()
    );
}
