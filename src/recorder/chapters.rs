//! User-reviewed chapter markers exported as FFmpeg-compatible metadata.
use anyhow::{Result, ensure};
use std::time::Duration;

pub const MAX_CHAPTERS: usize = 256;
pub const MAX_TITLE_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub start_ms: u64,
    pub title: String,
}

pub fn push_marker(markers: &mut Vec<Marker>, at: Duration) -> bool {
    if markers.len() >= MAX_CHAPTERS {
        return false;
    }
    let start_ms = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
    if markers
        .last()
        .is_some_and(|previous| previous.start_ms >= start_ms)
    {
        return false;
    }
    markers.push(Marker {
        start_ms,
        title: format!("第{}章", markers.len() + 1),
    });
    true
}

pub fn timestamp(start_ms: u64) -> String {
    let hours = start_ms / 3_600_000;
    let minutes = start_ms / 60_000 % 60;
    let seconds = start_ms / 1_000 % 60;
    let millis = start_ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

pub fn ffmetadata(markers: &[Marker], duration: Duration) -> Result<String> {
    ensure!(!markers.is_empty(), "没有可导出的章节标记");
    ensure!(markers.len() <= MAX_CHAPTERS, "章节数量超过上限");
    let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    let included: Vec<_> = markers
        .iter()
        .filter(|marker| marker.start_ms < duration_ms)
        .collect();
    ensure!(!included.is_empty(), "所有章节都位于视频时长之外");
    ensure!(
        included
            .windows(2)
            .all(|pair| pair[0].start_ms < pair[1].start_ms),
        "章节时间必须严格递增"
    );

    let mut output = String::from(";FFMETADATA1\n");
    for (index, marker) in included.iter().enumerate() {
        let end_ms = included
            .get(index + 1)
            .map_or(duration_ms, |next| next.start_ms);
        ensure!(end_ms > marker.start_ms, "章节时长必须大于零");
        ensure!(
            marker.title.len() <= MAX_TITLE_BYTES,
            "章节标题超过 512 字节"
        );
        ensure!(
            !marker
                .title
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\r' | '\n' | '\t')),
            "章节标题包含不支持的控制字符"
        );
        output.push_str("[CHAPTER]\nTIMEBASE=1/1000\n");
        output.push_str(&format!("START={}\nEND={}\n", marker.start_ms, end_ms));
        output.push_str("title=");
        let title = marker.title.trim();
        if title.is_empty() {
            output.push_str(&format!("第{}章", index + 1));
        } else {
            output.push_str(&escape_value(title));
        }
        output.push('\n');
    }
    Ok(output)
}

fn escape_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => {
                escaped.push('\\');
                escaped.push('\n');
            }
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                escaped.push('\\');
                escaped.push('\n');
            }
            '=' | ';' | '#' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_monotonic_millisecond_bounded_and_named() {
        let mut markers = Vec::new();
        assert!(push_marker(&mut markers, Duration::from_millis(1200)));
        assert!(!push_marker(&mut markers, Duration::from_micros(1_200_900)));
        assert!(push_marker(&mut markers, Duration::from_millis(2300)));
        assert_eq!(markers[0].title, "第1章");
        assert_eq!(markers[1].title, "第2章");
        assert_eq!(markers[1].start_ms, 2300);
        assert_eq!(timestamp(3_723_456), "01:02:03.456");
    }

    #[test]
    fn metadata_uses_next_chapter_as_end_and_escapes_title_injection() {
        let markers = vec![
            Marker {
                start_ms: 1250,
                title: "Introduction".into(),
            },
            Marker {
                start_ms: 2400,
                title: "A=B;#\\\n[CHAPTER]".into(),
            },
        ];
        let text = ffmetadata(&markers, Duration::from_millis(5000)).unwrap();
        assert!(text.contains("START=1250\nEND=2400\ntitle=Introduction\n"));
        assert!(text.contains("START=2400\nEND=5000\ntitle=A\\=B\\;\\#\\\\\\\n[CHAPTER]\n"));
        assert_eq!(text.matches("[CHAPTER]").count(), 3); // Header plus two sections.
    }

    #[test]
    fn metadata_rejects_empty_zero_length_and_out_of_order_chapters() {
        assert!(ffmetadata(&[], Duration::from_secs(1)).is_err());
        assert!(
            ffmetadata(
                &[Marker {
                    start_ms: 1000,
                    title: "At end".into()
                }],
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            ffmetadata(
                &[
                    Marker {
                        start_ms: 2000,
                        title: "later".into()
                    },
                    Marker {
                        start_ms: 1000,
                        title: "earlier".into()
                    }
                ],
                Duration::from_secs(3)
            )
            .is_err()
        );
    }

    #[test]
    fn metadata_uses_ffmpeg_line_continuation_for_newlines_and_rejects_controls() {
        let text = ffmetadata(
            &[Marker {
                start_ms: 0,
                title: "line one\r\nline two".into(),
            }],
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(text.contains("title=line one\\\nline two\n"));
        assert!(
            ffmetadata(
                &[Marker {
                    start_ms: 0,
                    title: "unsafe\0title".into(),
                }],
                Duration::from_secs(1),
            )
            .is_err()
        );
    }
}
