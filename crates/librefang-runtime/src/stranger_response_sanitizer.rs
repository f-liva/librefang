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
    let kept: Vec<&str> = text
        .split('\n')
        .filter(|line| !is_leak_line(line))
        .collect();

    let joined = kept.join("\n");
    let trimmed = joined.trim();

    if trimmed.is_empty() {
        NEUTRAL_FALLBACK.to_string()
    } else {
        // Preserve any trailing newline if the original had one and the
        // result is non-empty.
        joined
    }
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
    fn keeps_mixed_inline_status_when_no_newline() {
        // Single-line mixed: the leak survives but chat doesn't break.
        // Documented limitation — see module docs.
        let input = "Buon pomeriggio! Risposta inviata a Federico.";
        let out = sanitize_stranger_response(input);
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
