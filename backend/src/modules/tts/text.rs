//! Preparing bot-generated HTML for a phonemiser.
//!
//! This bot's replies are full of emoji (`🔊`, `✅`, `❓`, ...), and the spike
//! that proved sherpa-onnx's Kokoro engine works on this crate also showed it
//! logging `Unknown token: ❓` for each one: the lexicon has no reading for
//! them. Nothing fails when that happens — the token is just silently dropped
//! mid-utterance — so filtering them out here is the difference between a
//! sentence that reads naturally and one with unpredictable holes in it.

use crate::utils::strip_html;

/// Unicode blocks stripped before handing text to the phonemiser.
///
/// Each one is a block sherpa-onnx's Kokoro lexicon has no reading for:
/// emoticons, symbols and pictographs, transport symbols, misc symbols, and
/// dingbats. Variation selectors (`FE0F` and friends) ride along with base
/// emoji like `❤️` and are removed the same way.
const EMOJI_RANGES: &[(char, char)] = &[
    ('\u{1F300}', '\u{1F5FF}'), // Symbols and pictographs
    ('\u{1F600}', '\u{1F64F}'), // Emoticons
    ('\u{1F680}', '\u{1F6FF}'), // Transport symbols
    ('\u{2600}', '\u{26FF}'),   // Misc symbols
    ('\u{2700}', '\u{27BF}'),   // Dingbats
    ('\u{FE00}', '\u{FE0F}'),   // Variation selectors
];

/// Joins multi-codepoint emoji (e.g. a family emoji) into one glyph. It carries
/// no reading of its own and is not inside any range above.
const ZERO_WIDTH_JOINER: char = '\u{200D}';

fn is_unspeakable(c: char) -> bool {
    c == ZERO_WIDTH_JOINER || EMOJI_RANGES.iter().any(|&(lo, hi)| c >= lo && c <= hi)
}

/// Strips HTML tags and emoji from a bot reply, then collapses whitespace, so
/// the result is plain, phonemisable prose in whatever language it was in.
pub fn speakable(html: &str) -> String {
    let plain = strip_html(html);
    let without_emoji: String = plain.chars().filter(|&c| !is_unspeakable(c)).collect();

    without_emoji
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_html_tags_and_emoji_from_a_realistic_reply() {
        let html = "✅ <b>Added task</b> <i>Review budget</i> to your to-do list.<br/>🎉";
        let result = speakable(html);
        assert_eq!(result, "Added task Review budget to your to-do list.");
    }

    #[test]
    fn test_removes_a_multi_codepoint_emoji_joined_with_zwj() {
        // Family emoji: man + ZWJ + woman + ZWJ + girl, all inside the
        // symbols-and-pictographs block, joined by U+200D.
        let html = "Dinner's ready \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} come eat!";
        let result = speakable(html);
        assert_eq!(result, "Dinner's ready come eat!");
        assert!(!result.contains('\u{200D}'));
    }

    #[test]
    fn test_leaves_chinese_and_english_intact() {
        let html = "早安。你今天有 <b>3</b> 件待办事项。Good morning!";
        let result = speakable(html);
        assert_eq!(result, "早安。你今天有 3 件待办事项。Good morning!");
    }

    #[test]
    fn test_collapses_whitespace_and_trims() {
        let html = "  <p>Line one</p>\n\n  <p>Line two</p>  ";
        let result = speakable(html);
        assert_eq!(result, "Line one Line two");
    }

    #[test]
    fn test_empty_and_emoji_only_input_yields_empty_string() {
        assert_eq!(speakable(""), "");
        assert_eq!(speakable("✅🎉❓"), "");
    }
}
