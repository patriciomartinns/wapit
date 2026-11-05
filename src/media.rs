use crate::graph_error::GraphError;

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;

/// The largest media file Meta accepts, a document.
pub const MAX_MEDIA_BYTES: u64 = 100 * MB;

struct MediaType {
    kind: &'static str,
    mime: &'static str,
    max_bytes: u64,
}

/// Meta's supported media types. Stickers share `image/webp` with images; an animated sticker may be 500 KB,
/// which wapit uses as the sticker limit because it cannot tell animated from static.
const TYPES: &[MediaType] = &[
    MediaType {
        kind: "audio",
        mime: "audio/aac",
        max_bytes: 16 * MB,
    },
    MediaType {
        kind: "audio",
        mime: "audio/amr",
        max_bytes: 16 * MB,
    },
    MediaType {
        kind: "audio",
        mime: "audio/mpeg",
        max_bytes: 16 * MB,
    },
    MediaType {
        kind: "audio",
        mime: "audio/mp4",
        max_bytes: 16 * MB,
    },
    MediaType {
        kind: "audio",
        mime: "audio/ogg",
        max_bytes: 16 * MB,
    },
    MediaType {
        kind: "document",
        mime: "text/plain",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/vnd.ms-excel",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/msword",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/vnd.ms-powerpoint",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "document",
        mime: "application/pdf",
        max_bytes: 100 * MB,
    },
    MediaType {
        kind: "image",
        mime: "image/jpeg",
        max_bytes: 5 * MB,
    },
    MediaType {
        kind: "image",
        mime: "image/png",
        max_bytes: 5 * MB,
    },
    MediaType {
        kind: "image",
        mime: "image/webp",
        max_bytes: 5 * MB,
    },
    MediaType {
        kind: "sticker",
        mime: "image/webp",
        max_bytes: 500 * KB,
    },
    MediaType {
        kind: "video",
        mime: "video/3gpp",
        max_bytes: 16 * MB,
    },
    MediaType {
        kind: "video",
        mime: "video/mp4",
        max_bytes: 16 * MB,
    },
];

/// The MIME type without parameters, so `audio/ogg; codecs=opus` reads as `audio/ogg`.
pub fn essence(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

fn megabytes(bytes: u64) -> String {
    if bytes >= MB {
        format!("{} MB", bytes.div_ceil(MB))
    } else {
        format!("{} KB", bytes.div_ceil(KB))
    }
}

/// Checks a `POST /{phone_number_id}/media` upload against the types and sizes Meta supports.
pub fn check_upload(mime: &str, size: u64) -> Result<(), GraphError> {
    let mime = essence(mime);
    let largest = TYPES
        .iter()
        .filter(|candidate| candidate.mime == mime)
        .map(|candidate| candidate.max_bytes)
        .max();

    let Some(largest) = largest else {
        let mut supported: Vec<&str> = TYPES.iter().map(|candidate| candidate.mime).collect();
        supported.dedup();

        return Err(GraphError::new(
            100,
            format!(
                "(#100) Param file must be a file with one of the following types: {}. Received file of type '{mime}'.",
                supported.join(", ")
            ),
        ));
    };

    if size > largest {
        return Err(
            GraphError::new(131_053, "(#131053) Media upload error").details(format!(
                "{mime} files can be at most {}, this one is {}",
                megabytes(largest),
                megabytes(size)
            )),
        );
    }

    Ok(())
}

/// Whether an uploaded file can be sent as a `kind` message: its type belongs to that kind and fits its limit.
pub fn fits(kind: &str, mime: &str, size: u64) -> bool {
    let mime = essence(mime);

    TYPES.iter().any(|candidate| {
        candidate.kind == kind && candidate.mime == mime && size <= candidate.max_bytes
    })
}

#[cfg(test)]
mod tests {
    use super::{MB, check_upload, fits};

    #[test]
    fn uploads_follow_metas_types_and_sizes() {
        assert!(check_upload("image/png", 5 * MB).is_ok());
        assert_eq!(
            check_upload("image/png", 5 * MB + 1).unwrap_err().code,
            131_053
        );
        assert!(check_upload("audio/ogg; codecs=opus", MB).is_ok());
        assert!(check_upload("application/pdf", 100 * MB).is_ok());
        assert_eq!(check_upload("text/html", 10).unwrap_err().code, 100);
    }

    #[test]
    fn a_file_fits_only_its_own_kind() {
        assert!(fits("image", "image/webp", 2 * MB));
        assert!(!fits("sticker", "image/webp", 2 * MB));
        assert!(!fits("video", "image/png", 10));
        assert!(fits("document", "text/plain", 10));
    }
}
