//! Post-LLM sanitizer for stranger-channel responses.
//!
//! Strips status/meta leak patterns that reveal owner-side operations
//! (e.g. "Risposta inviata", "Ho notificato il Signore in privato")
//! from text the agent is about to send back to a non-owner sender.
//!
//! The agent's prompt is supposed to forbid these patterns, but in practice
//! the LLM still emits them. This module enforces the boundary at the runtime
//! level so the stranger never sees internal operational status.

/// Patterns that, when present at the start of a line (case-insensitive,
/// after trimming whitespace), cause that line to be dropped from the
/// stranger response.
const LEAK_LINE_PREFIXES: &[&str] = &[
    "risposta inviata",
    "messaggio inviato",
    "il messaggio è stato recapitato",
    "il messaggio e' stato recapitato",
    "il messaggio è stato consegnato",
    "il messaggio e' stato consegnato",
    "il contatto è stato risposto",
    "il contatto e' stato risposto",
    "il signore è stato notificato",
    "il signore e' stato notificato",
    "ho notificato il signore",
    "ho inoltrato la richiesta",
    "ho inoltrato al signore",
    "ho inoltrato la tua richiesta",
    "ho girato la richiesta",
    "ho girato al signore",
    "ho trasmesso al signore",
    "ho comunicato al signore",
    "ho passato al signore",
    "ho avvisato il signore",
    "ho consegnato il messaggio",
    "ho recapitato",
    "ho fatto sapere al signore",
    "ho risposto al contatto",
    "ho risposto allo stranger",
    "conferma invio",
    "conferma inviata",
    "invio completato",
    "inoltro completato",
    "risposta consegnata",
    "ho già inoltrato",
    "ho gia' inoltrato",
    "l'ho fatto presente",
    // "proprietario" / "padrone" / "datore" synonyms for "Signore"
    "il proprietario è stato notificato",
    "il proprietario e' stato notificato",
    "il proprietario è stato avvisato",
    "il proprietario e' stato avvisato",
    "ho notificato il proprietario",
    "ho avvisato il proprietario",
    "proprietario notificato",
    "il padrone è stato notificato",
    "il padrone e' stato notificato",
    "ho notificato il padrone",
    "il datore è stato notificato",
    "il datore e' stato notificato",
    "ho notificato il datore",
];

/// Used when the entire response collapses to nothing after sanitization.
/// Better to send a neutral acknowledgement than empty text (which would
/// confuse the stranger and possibly trigger the WhatsApp gateway's
/// empty-message guard).
const NEUTRAL_FALLBACK: &str = "Va bene. 🎩";

/// Sanitize a response that is about to be sent to a stranger
/// (non-owner) channel. Returns the cleaned text.
///
/// Behavior:
/// - Splits input on newlines
/// - Drops any line whose trimmed lowercased content starts with a leak prefix
/// - Re-joins remaining lines with `\n`
/// - If the result is empty or whitespace-only, returns the neutral fallback
///
/// Note: the function intentionally does NOT split on sentence boundaries
/// (".", "!", "?"). Many natural responses share a line with a status meta
/// sentence (e.g. "Buon pomeriggio! Risposta inviata."). For those, dropping
/// the line keeps the chat coherent — the stranger still gets a saluto via
/// the leading natural reply emitted on its own line. If the LLM emits a
/// single-line mixed response, the sanitizer keeps it untouched (acceptable
/// fallback: the leak survives but the chat doesn't break).
pub fn sanitize_stranger_response(text: &str) -> String {
    // Per line:
    // - if originally blank → preserve as empty string (keeps user's spacing)
    // - else sanitize at sentence level; if result is empty/blank → drop the
    //   whole line so we don't leave gaps where leak lines used to be
    let kept: Vec<String> = text
        .split('\n')
        .filter_map(|line| {
            if line.trim().is_empty() {
                Some(String::new())
            } else {
                let s = sanitize_line(line);
                if s.trim().is_empty() {
                    None
                } else {
                    Some(s)
                }
            }
        })
        .collect();

    let joined = kept.join("\n");
    if joined.trim().is_empty() {
        NEUTRAL_FALLBACK.to_string()
    } else {
        joined
    }
}

/// Within a single line, split into sentence-like fragments on terminators
/// followed by whitespace + uppercase start, and drop any fragment whose
/// trimmed lowercase prefix matches a leak pattern.
fn sanitize_line(line: &str) -> String {
    if is_leak_line(line) {
        return String::new();
    }
    if line.trim().is_empty() {
        return line.to_string();
    }

    let sentences = split_into_sentences(line);
    let kept: Vec<&str> = sentences
        .iter()
        .copied()
        .filter(|s| !is_leak_sentence(s))
        .collect();

    if kept.len() == sentences.len() {
        // No sentence dropped → return original line untouched (preserve
        // any quirky whitespace the LLM produced).
        return line.to_string();
    }

    // Rebuild line from kept sentences. Each sentence retains its own
    // terminator, so a single space between them yields natural prose.
    kept.iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Split a single line into sentence-like substrings.
///
/// A sentence boundary is one of `.`, `!`, `?` followed by one or more
/// ASCII whitespace characters and an uppercase letter (Latin set,
/// including common Italian accented capitals). This heuristic avoids
/// splitting on abbreviations like "Sig. Federico" when "Federico"
/// starts with uppercase — that case stays one fragment because it
/// is rare in stranger replies and harmless to keep grouped.
fn split_into_sentences(line: &str) -> Vec<&str> {
    let bytes = line.as_bytes();
    let mut starts: Vec<usize> = vec![0];

    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if matches!(c, b'.' | b'!' | b'?') {
            // Scan whitespace after terminator
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j < bytes.len() && is_sentence_start_byte(&bytes[j..]) && j > i + 1 {
                // Boundary: next sentence starts at j
                starts.push(j);
                i = j;
                continue;
            }
        }
        i += 1;
    }

    let mut out: Vec<&str> = Vec::with_capacity(starts.len());
    for w in 0..starts.len() {
        let start = starts[w];
        let end = starts.get(w + 1).copied().unwrap_or(bytes.len());
        out.push(&line[start..end]);
    }
    out
}

/// Returns true if the byte slice begins with a character that looks
/// like the start of a new sentence (uppercase Latin, including common
/// Italian accented capitals encoded as 2-byte UTF-8).
fn is_sentence_start_byte(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let b0 = bytes[0];
    // ASCII uppercase A-Z
    if b0.is_ascii_uppercase() {
        return true;
    }
    // Common Italian accented uppercase letters in UTF-8 (2-byte prefix
    // 0xC3 followed by 0x80-0x9E covers À..Ý etc.)
    if bytes.len() >= 2 && b0 == 0xC3 {
        let b1 = bytes[1];
        // 0x80..=0x9E covers uppercase Latin-1 supplement
        if (0x80..=0x9E).contains(&b1) {
            return true;
        }
    }
    false
}

/// Returns true if the sentence (after trim + lowercase) starts with
/// any known leak prefix.
fn is_leak_sentence(s: &str) -> bool {
    let lc = s.trim().to_lowercase();
    if lc.is_empty() {
        return false;
    }
    LEAK_LINE_PREFIXES
        .iter()
        .any(|prefix| lc.starts_with(prefix))
}

/// Returns true if `line` (after trim + lowercase) starts with any of the
/// configured leak prefixes.
fn is_leak_line(line: &str) -> bool {
    let lc = line.trim().to_lowercase();
    if lc.is_empty() {
        return false;
    }
    LEAK_LINE_PREFIXES
        .iter()
        .any(|prefix| lc.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_clean_text() {
        let input = "Ciao Federico! Tutto bene, grazie. E tu come stai?";
        assert_eq!(sanitize_stranger_response(input), input);
    }

    #[test]
    fn drops_risposta_inviata_line() {
        let input = "Ciao Federico! Piacere di conoscerti. Un saluto anche a te 🎩\nRisposta inviata a Federico.";
        let out = sanitize_stranger_response(input);
        assert_eq!(
            out,
            "Ciao Federico! Piacere di conoscerti. Un saluto anche a te 🎩"
        );
    }

    #[test]
    fn drops_ho_notificato_il_signore_line() {
        let input = "Buon pomeriggio.\nHo notificato il Signore in privato della richiesta.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Buon pomeriggio.");
    }

    #[test]
    fn drops_il_messaggio_e_stato_recapitato() {
        let input = "Va bene, ti ringrazio.\nIl messaggio è stato recapitato al contatto.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Va bene, ti ringrazio.");
    }

    #[test]
    fn drops_ho_girato_la_richiesta_line() {
        let input =
            "Ciao Federico!\nHo girato la richiesta al Signore e ti farò sapere.\nA presto!";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Ciao Federico!\nA presto!");
    }

    #[test]
    fn drops_il_contatto_e_stato_risposto() {
        let input = "Ok.\nIl contatto è stato risposto: gli ho comunicato che verificherò la disponibilità.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Ok.");
    }

    #[test]
    fn drops_multiple_leak_lines() {
        let input = "Ciao!\nRisposta inviata.\nHo notificato il Signore.\nGrazie.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Ciao!\nGrazie.");
    }

    #[test]
    fn returns_neutral_fallback_when_only_leak() {
        let input = "Risposta inviata a Federico.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, NEUTRAL_FALLBACK);
    }

    #[test]
    fn returns_neutral_fallback_when_all_lines_leak() {
        let input =
            "Risposta inviata.\nHo notificato il Signore.\nIl messaggio è stato recapitato.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, NEUTRAL_FALLBACK);
    }

    #[test]
    fn returns_neutral_fallback_when_empty() {
        assert_eq!(sanitize_stranger_response(""), NEUTRAL_FALLBACK);
        assert_eq!(sanitize_stranger_response("   \n  \n  "), NEUTRAL_FALLBACK);
    }

    #[test]
    fn case_insensitive_match() {
        let input = "RISPOSTA INVIATA a Federico.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, NEUTRAL_FALLBACK);
    }

    #[test]
    fn leading_whitespace_does_not_save_leak_line() {
        let input = "Ciao!\n    Risposta inviata a Federico.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Ciao!");
    }

    #[test]
    fn drops_inline_leak_sentence_after_natural_one() {
        let input = "Buon pomeriggio! Risposta inviata a Federico.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Buon pomeriggio!");
    }

    #[test]
    fn drops_proprietario_synonym_inline() {
        let input = "Non riesco a recuperare il JID del contatto. Il proprietario è stato notificato con successo della richiesta.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Non riesco a recuperare il JID del contatto.");
    }

    #[test]
    fn drops_proprietario_padrone_datore_variants() {
        for v in &[
            "Il proprietario è stato notificato.",
            "Il padrone è stato notificato.",
            "Ho notificato il proprietario.",
            "Ho avvisato il proprietario.",
            "Il datore è stato notificato.",
        ] {
            let input = format!("Ciao. {v}");
            let out = sanitize_stranger_response(&input);
            assert_eq!(out, "Ciao.", "failed for variant: {v}");
        }
    }

    #[test]
    fn keeps_natural_inline_when_no_leak() {
        let input = "Buon pomeriggio! Come posso aiutarLa?";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, input);
    }

    #[test]
    fn keeps_lowercase_continuation_grouped() {
        // No split on "Sig. Federico" — lowercase after dot keeps fragment whole
        let input = "Salve Sig. federico, ho ricevuto.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, input);
    }

    #[test]
    fn handles_accented_uppercase_start() {
        let input = "Ho controllato. È disponibile.";
        let out = sanitize_stranger_response(input);
        // No leak → unchanged
        assert_eq!(out, input);
    }

    #[test]
    fn drops_apostrophe_variants() {
        let input = "Ok.\nIl messaggio e' stato recapitato.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Ok.");
    }

    #[test]
    fn drops_ho_inoltrato_variants() {
        for variant in &[
            "Ho inoltrato la richiesta al Signore.",
            "Ho inoltrato la tua richiesta",
            "Ho inoltrato al Signore le tue parole.",
        ] {
            let input = format!("Ciao!\n{variant}");
            let out = sanitize_stranger_response(&input);
            assert_eq!(out, "Ciao!", "failed for variant: {variant}");
        }
    }

    #[test]
    fn drops_ho_gia_inoltrato_with_accent() {
        let input = "Va bene.\nHo già inoltrato la tua richiesta.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, "Va bene.");
    }

    #[test]
    fn ignores_blank_lines_in_filter() {
        let input = "Ciao!\n\nGrazie.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_natural_text_containing_leak_words_inline() {
        // "Risposta" appears inside natural prose, not as line prefix.
        let input = "Aspetto la tua risposta inviata via email, grazie.";
        let out = sanitize_stranger_response(input);
        assert_eq!(out, input);
    }
}
