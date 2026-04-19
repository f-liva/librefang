//! HTML tag + entity stripper for plain-text fallback paths.
//!
//! Used by channel adapters (Telegram, primarily) as a last-resort
//! sanitizer when rich-text parsing fails server-side and we must ship
//! clean plain text instead of literal tag characters.
//!
//! This is NOT a general-purpose sanitizer. It intentionally:
//! - strips ALL tags (no allowlist — the whole point is "clean at any cost")
//! - decodes common HTML entities via `html_escape::decode_html_entities`
//! - preserves newlines verbatim (no block-level reflow)
//! - discards tag attributes (URLs in `<a href="…">` are LOST — the
//!   fallback path optimises for visible cleanliness, not feature parity)
//!
//! Incident reference (2026-04-19): Telegram rejected a message containing
//! crossed HTML tags `<b>797 <i>Aira</b></i>` with HTTP 400 "can't parse
//! entities"; the retry-fallback branch in `telegram::api_send_message`
//! re-shipped the SAME chunk without `parse_mode`, which Telegram then
//! rendered as plain text — so literal `<b>`/`</b>`/`<i>`/`</i>` characters
//! appeared in the delivered message. This module prevents that failure
//! mode by stripping tags before the plain retry.

/// Strip ALL HTML tags and decode common HTML entities from `text`.
///
/// Tags are identified purely structurally (`<…>` span) — there is no
/// allowlist. Unknown, malformed, or crossed tags are all stripped.
/// Tag attributes are discarded (e.g. `<a href="url">label</a>` → `label`).
///
/// Newlines inside the input are preserved verbatim. Leading/trailing
/// whitespace is NOT trimmed (caller decides).
///
/// Entity decoding is handled by `html_escape::decode_html_entities`,
/// which covers named, decimal, and hex entities (`&amp;`, `&lt;`, `&gt;`,
/// `&quot;`, `&#39;`, `&apos;`, `&nbsp;`, plus the full HTML5 named table).
pub(crate) fn strip_html_for_fallback(text: &str) -> String {
    // Char-by-char walker: toggle `in_tag` between '<' and '>' boundaries.
    // Non-tag characters accumulate into `result`; tag characters are dropped.
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => {
                in_tag = true;
            }
            '>' if in_tag => {
                in_tag = false;
            }
            _ if in_tag => { /* drop tag content */ }
            _ => {
                result.push(ch);
            }
        }
    }

    // Decode common entities. `html_escape` handles named, decimal, hex.
    html_escape::decode_html_entities(&result).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_simple_bold() {
        assert_eq!(strip_html_for_fallback("<b>hello</b>"), "hello");
    }

    #[test]
    fn test_strip_crossed_tags_incident() {
        // The exact shape that broke production 2026-04-19.
        assert_eq!(
            strip_html_for_fallback("<b>797 <i>Aira</b></i>"),
            "797 Aira"
        );
    }

    #[test]
    fn test_strip_anchor_discards_url() {
        assert_eq!(
            strip_html_for_fallback("<a href=\"http://x\">link</a>"),
            "link"
        );
    }

    #[test]
    fn test_strip_decodes_entities() {
        assert_eq!(
            strip_html_for_fallback("AT&amp;T says &lt;hello&gt;"),
            "AT&T says <hello>"
        );
    }

    #[test]
    fn test_strip_preserves_newlines() {
        assert_eq!(strip_html_for_fallback("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn test_strip_empty() {
        assert_eq!(strip_html_for_fallback(""), "");
    }

    #[test]
    fn test_strip_pass_through_no_tags() {
        assert_eq!(strip_html_for_fallback("no tags here"), "no tags here");
    }

    #[test]
    fn test_strip_nested_code_blocks() {
        assert_eq!(
            strip_html_for_fallback("<pre><code>fn x()</code></pre>"),
            "fn x()"
        );
    }

    #[test]
    fn test_strip_quote_and_apos_entities() {
        // &nbsp; decodes to U+00A0 (non-breaking space) via html_escape.
        // We accept either ASCII space or NBSP for this fallback path; the
        // assertion focuses on the quote/apos entities which ARE ASCII.
        let result = strip_html_for_fallback("a &quot;b&quot; &#39;c&#39;");
        assert_eq!(result, "a \"b\" 'c'");
    }

    #[test]
    fn test_strip_unknown_tags_no_allowlist() {
        assert_eq!(
            strip_html_for_fallback("<unknown-tag>text</unknown-tag>"),
            "text"
        );
    }

    #[test]
    fn test_strip_mixed_tags_and_text() {
        assert_eq!(
            strip_html_for_fallback("before <b>bold</b> middle <i>italic</i> after"),
            "before bold middle italic after"
        );
    }

    #[test]
    fn test_strip_attributes_with_quotes() {
        // Attribute values containing '>' would confuse us, but valid HTML
        // shouldn't have raw '>' inside attribute values — they'd be &gt;.
        assert_eq!(
            strip_html_for_fallback("<span class=\"big\">visible</span>"),
            "visible"
        );
    }

    #[test]
    fn test_strip_self_closing_tag() {
        assert_eq!(strip_html_for_fallback("line1<br/>line2"), "line1line2");
    }

    #[test]
    fn test_strip_lone_open_bracket_no_close() {
        // Unclosed `<` swallows everything until EOF per the in_tag state.
        // This is acceptable for the fallback path — the caller's input
        // should come from well-formed Telegram HTML even if nesting is
        // crossed.
        assert_eq!(strip_html_for_fallback("before <still in tag"), "before ");
    }
}
