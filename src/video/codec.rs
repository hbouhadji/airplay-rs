use std::borrow::Cow;

#[derive(Debug, Clone, Copy)]
pub(super) enum Codec {
    H264,
    H265,
}

impl Codec {
    pub(super) fn normalize_codec_data<'a>(self, codec_data: &'a [u8]) -> Cow<'a, [u8]> {
        match self {
            Self::H264 => Cow::Borrowed(codec_data),
            Self::H265 => find_mp4_box_payload(codec_data, b"hvcC")
                .map(Cow::Borrowed)
                .unwrap_or(Cow::Borrowed(codec_data)),
        }
    }
}

fn find_mp4_box_payload<'a>(payload: &'a [u8], box_type: &[u8; 4]) -> Option<&'a [u8]> {
    if payload.len() < 8 {
        return None;
    }

    for offset in 0..=payload.len().saturating_sub(8) {
        if &payload[offset + 4..offset + 8] != box_type {
            continue;
        }

        let size = u32::from_be_bytes(payload[offset..offset + 4].try_into().ok()?) as usize;
        if size < 8 {
            continue;
        }

        let box_end = offset.checked_add(size)?;
        if box_end > payload.len() {
            continue;
        }

        return Some(&payload[offset + 8..box_end]);
    }

    None
}
