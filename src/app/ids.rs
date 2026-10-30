//! Process-independent random identifiers; no global counters or cached epochs.
pub fn nonce() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("OS random source unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
