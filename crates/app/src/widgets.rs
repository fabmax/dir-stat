//! Small view helpers. iced 0.14 text has no ellipsis, so long text is pre-truncated by
//! estimated width (UX §12.6) and clipped as a safety net.

/// Average glyph width of Inter, as a share of the font size.
const GLYPH_EM: f32 = 0.56;

pub fn estimated_width(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * size * GLYPH_EM
}

fn max_chars(max_px: f32, size: f32) -> usize {
    (max_px / (size * GLYPH_EM)).floor().max(0.0) as usize
}

/// "Application Supp…"
pub fn truncate_end(s: &str, max_px: f32, size: f32) -> String {
    let max = max_chars(max_px, size);
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// Paths keep their first component and the last ones that fit:
/// "Library/…/Caches/com.apple.Safari". Falls back to an end-cut of the last component.
pub fn truncate_middle(path: &str, max_px: f32, size: f32) -> String {
    let max = max_chars(max_px, size);
    if path.chars().count() <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 3 {
        let first = parts[0];
        for keep in (1..parts.len() - 1).rev() {
            let tail = parts[parts.len() - keep..].join("/");
            let candidate = format!("{first}/…/{tail}");
            if candidate.chars().count() <= max {
                return candidate;
            }
        }
        let last = parts[parts.len() - 1];
        let candidate = format!("…/{last}");
        if candidate.chars().count() <= max {
            return candidate;
        }
    }
    let chars: Vec<char> = path.chars().collect();
    let keep = max.saturating_sub(1);
    let tail: String = chars[chars.len().saturating_sub(keep)..].iter().collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(chars: usize) -> f32 {
        chars as f32 * 10.0 * GLYPH_EM + 0.01
    }

    #[test]
    fn end_truncation() {
        assert_eq!(
            truncate_end("Application Support", px(17), 10.0),
            "Application Supp…"
        );
        assert_eq!(truncate_end("short", px(17), 10.0), "short");
    }

    #[test]
    fn middle_truncation_keeps_first_and_last_components() {
        let path = "Library/Caches/com.apple.Safari/fsCachedData";
        assert_eq!(truncate_middle(path, px(60), 10.0), path);
        assert_eq!(
            truncate_middle(path, px(40), 10.0),
            "Library/…/com.apple.Safari/fsCachedData"
        );
        assert_eq!(
            truncate_middle(path, px(25), 10.0),
            "Library/…/fsCachedData"
        );
        assert_eq!(truncate_middle(path, px(14), 10.0), "…/fsCachedData");
        assert_eq!(truncate_middle(path, px(8), 10.0), "…hedData");
    }
}
