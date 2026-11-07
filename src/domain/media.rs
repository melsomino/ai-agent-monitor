//! Bounded image payloads published by agent adapters; human channels need no transcript access.
use std::sync::Arc;
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub id: String,
    pub mime: &'static str,
    pub data: Arc<[u8]>,
}

impl Image {
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        if self.mime == "image/png" {
            let size = self.data.get(16..24)?;
            return Some((
                u32::from_be_bytes(size[..4].try_into().ok()?),
                u32::from_be_bytes(size[4..].try_into().ok()?),
            ));
        }
        if self.mime != "image/jpeg" {
            return None;
        }
        let data = &self.data;
        let mut p = 2;
        while p < data.len() {
            if *data.get(p)? != 0xff {
                return None;
            }
            while data.get(p) == Some(&0xff) {
                p += 1;
            }
            let marker = *data.get(p)?;
            p += 1;
            if matches!(marker, 0xd8 | 0x01 | 0xd0..=0xd7) {
                continue;
            }
            if matches!(marker, 0xd9 | 0xda) {
                return None;
            }
            let len = u16::from_be_bytes(data.get(p..p + 2)?.try_into().ok()?) as usize;
            if len < 2 || p.checked_add(len)? > data.len() {
                return None;
            }
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                if len < 7 {
                    return None;
                }
                let h = u16::from_be_bytes(data.get(p + 3..p + 5)?.try_into().ok()?);
                let w = u16::from_be_bytes(data.get(p + 5..p + 7)?.try_into().ok()?);
                return Some((u32::from(w), u32::from(h)));
            }
            p += len;
        }
        None
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("id", &self.id)
            .field("mime", &self.mime)
            .field("bytes", &self.data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jpeg_dimensions_are_bounded_and_truncated_headers_are_not_photos() {
        let bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 16, 0, 32, 1, 1, 17, 0];
        let image = Image {
            id: "test".into(),
            mime: "image/jpeg",
            data: bytes.clone().into(),
        };
        assert_eq!(image.dimensions(), Some((32, 16)));
        for n in 0..bytes.len() {
            let image = Image {
                id: "test".into(),
                mime: "image/jpeg",
                data: bytes[..n].into(),
            };
            assert_eq!(image.dimensions(), None);
        }
    }
}
