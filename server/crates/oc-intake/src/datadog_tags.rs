//! The Datadog Agent's encoded tag sets in `CollectorConnections` (`encodedTags`,
//! `encodedConnectionsTags`): many tag lists in one buffer, each addressed by an index.
//!
//! Layout from `DataDog/agent-payload`, `process/tags.go` and `process/tags_v2.go` (BSD-3-Clause,
//! Copyright (c) 2017 Datadog, Inc.):
//! - version 1: `[1]`, then per list a u16 tag count and, per tag, a u16 length and its bytes; an
//!   index is the list's byte offset in the buffer;
//! - versions 2 and 3: `[2|3]`, a u32 footer position, every distinct tag once (u16 length and
//!   bytes), then the footer: per list a u16 count and u32 positions of its tags; an index is the
//!   list's offset in the footer.
//!
//! Integers are little-endian.

fn u16_at(buf: &[u8], pos: usize) -> Option<usize> {
    Some(u16::from_le_bytes(buf.get(pos..pos + 2)?.try_into().ok()?) as usize)
}

fn u32_at(buf: &[u8], pos: usize) -> Option<usize> {
    Some(u32::from_le_bytes(buf.get(pos..pos + 4)?.try_into().ok()?) as usize)
}

/// A u16-length-prefixed string at `pos`.
fn tag_at(buf: &[u8], pos: usize) -> Option<String> {
    let len = u16_at(buf, pos)?;
    String::from_utf8(buf.get(pos + 2..pos + 2 + len)?.to_vec()).ok()
}

/// The tag list at `index`; empty when the index or buffer is invalid.
pub fn tags_at(buf: &[u8], index: i32) -> Vec<String> {
    let Ok(index) = usize::try_from(index) else { return Vec::new() };
    let decode = || -> Option<Vec<String>> {
        match *buf.first()? {
            1 => {
                // Index 0 is the version byte, never a list.
                if index == 0 {
                    return None;
                }
                let n = u16_at(buf, index)?;
                let mut pos = index + 2;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let t = tag_at(buf, pos)?;
                    pos += 2 + t.len();
                    out.push(t);
                }
                Some(out)
            }
            2 | 3 => {
                let footer = u32_at(buf, 1)?.checked_add(index)?;
                let n = u16_at(buf, footer)?;
                (0..n).map(|i| u32_at(buf, footer + 2 + i * 4).and_then(|p| tag_at(buf, p))).collect()
            }
            _ => None,
        }
    };
    decode().unwrap_or_default()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Encodes tag lists the way the Agent's V2 encoder does; returns the buffer and each list's index.
    pub fn encode_v2(lists: &[&[&str]]) -> (Vec<u8>, Vec<i32>) {
        let mut order: Vec<&str> = Vec::new();
        let mut positions = std::collections::HashMap::new();
        let mut pos = 5u32;
        let mut footer = Vec::new();
        let mut indexes = Vec::new();
        for list in lists {
            indexes.push(footer.len() as i32);
            footer.extend_from_slice(&(list.len() as u16).to_le_bytes());
            for t in *list {
                let p = *positions.entry(*t).or_insert_with(|| {
                    order.push(t);
                    let p = pos;
                    pos += 2 + t.len() as u32;
                    p
                });
                footer.extend_from_slice(&p.to_le_bytes());
            }
        }
        let mut buf = vec![2u8];
        buf.extend_from_slice(&pos.to_le_bytes());
        for t in order {
            buf.extend_from_slice(&(t.len() as u16).to_le_bytes());
            buf.extend_from_slice(t.as_bytes());
        }
        buf.extend_from_slice(&footer);
        (buf, indexes)
    }

    #[test]
    fn v2_lists_share_tags() {
        let (buf, idx) = encode_v2(&[&["env:prod", "service:checkout"], &["env:prod", "kube_namespace:shop"]]);
        assert_eq!(tags_at(&buf, idx[0]), ["env:prod", "service:checkout"]);
        assert_eq!(tags_at(&buf, idx[1]), ["env:prod", "kube_namespace:shop"]);
        assert!(tags_at(&buf, -1).is_empty());
        assert!(tags_at(&buf, 9999).is_empty());
    }

    #[test]
    fn v1_lists() {
        let mut buf = vec![1u8];
        let index = buf.len() as i32;
        buf.extend_from_slice(&2u16.to_le_bytes());
        for t in ["env:prod", "team:web"] {
            buf.extend_from_slice(&(t.len() as u16).to_le_bytes());
            buf.extend_from_slice(t.as_bytes());
        }
        assert_eq!(tags_at(&buf, index), ["env:prod", "team:web"]);
        assert!(tags_at(&buf, 0).is_empty());
    }
}
