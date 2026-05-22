//! Rate-limit user notification.
//!
//! When the agent_loop exhausts its in-loop retry budget on an LLM
//! rate-limit / overload, the message is parked as `Deferred` in the
//! journal and silently re-dispatched later. From the sender's
//! perspective this looks like the agent simply ignored them — there
//! is no feedback that the message is queued and will be answered
//! when the quota window resets.
//!
//! This module produces a short user-facing notification that the
//! bridge sends BEFORE deferring, so the sender at least knows the
//! turn was received and is on hold.

use crate::message_journal::parse_defer_marker;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// Default user-facing notification template. Used when the operator has
/// not customized it via `LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE`. Matches
/// the text shape historically declared under
/// `[agents.defaults.rate_limit_notify].template` in config.toml.
///
/// Supported placeholders:
/// * `{reset_time}` — clock time when the upstream quota window resets,
///   rendered in the box's configured timezone (see [`box_tz`]).
pub const DEFAULT_TEMPLATE: &str =
    "⏸️ Limite quota raggiunto. Reset alle {reset_time}. Ti rispondo dopo.";

/// Read the active notification template. Looks first at
/// `LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE`; falls back to
/// [`DEFAULT_TEMPLATE`].
fn active_template() -> String {
    std::env::var("LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_TEMPLATE.to_string())
}

/// IANA timezone used to render the reset clock time. LibreFang's box
/// runs in Europe/Rome (`[system] timezone` in config.toml); we hardcode
/// it here because the channels crate currently has no path to the live
/// `[system]` config block. If the deployment moves, override via the
/// `LIBREFANG_TZ` env var (parsed at module load time).
fn box_tz() -> Tz {
    std::env::var("LIBREFANG_TZ")
        .ok()
        .and_then(|s| s.parse::<Tz>().ok())
        .unwrap_or(chrono_tz::Europe::Rome)
}

/// Build a notification text for a deferred turn, given the kernel
/// error string. Returns `None` if the error does not carry the
/// rate-limit defer marker (i.e. it's a real failure, not a quota
/// hold) or if defer_ms is implausibly small/zero.
///
/// `now` is injected so tests can pin the clock; production callers
/// pass `Utc::now()`.
pub fn build_rate_limit_notify_text(err_str: &str, now: DateTime<Utc>) -> Option<String> {
    let defer_ms = parse_defer_marker(err_str)?;
    if defer_ms < 1_000 {
        // Less than a second of defer: not worth notifying.
        return None;
    }
    let reset_at = now + chrono::Duration::milliseconds(defer_ms as i64);
    // Render in the box-local timezone (Europe/Rome assumed for this
    // deployment). Fall back to UTC if the named TZ can't be parsed.
    let reset_local = render_local_time(reset_at);
    let template = active_template();
    Some(template.replace("{reset_time}", &reset_local))
}

/// Render the given UTC time as HH:MM in the box's configured
/// timezone (see [`box_tz`]).
fn render_local_time(t: DateTime<Utc>) -> String {
    use chrono::offset::TimeZone;
    let tz = box_tz();
    let local = tz.from_utc_datetime(&t.naive_utc());
    local.format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 22, 9, 0, 0).unwrap()
    }

    #[test]
    fn returns_none_for_non_rate_limit_error() {
        let err = "Some other transport failure";
        assert!(build_rate_limit_notify_text(err, fixed_now()).is_none());
    }

    #[test]
    fn returns_none_for_zero_defer_ms() {
        let err = "Rate limited [rate_limit_defer_ms]=0";
        assert!(build_rate_limit_notify_text(err, fixed_now()).is_none());
    }

    #[test]
    fn returns_some_for_defer_marker() {
        // 5min = 300_000 ms; from 09:00 UTC + 5m = 09:05 UTC = 11:05 CEST
        std::env::set_var("LIBREFANG_TZ", "Europe/Rome");
        let err = "Rate limited after 3 retries [rate_limit_defer_ms]=300000";
        let out = build_rate_limit_notify_text(err, fixed_now()).expect("expected Some");
        assert!(out.contains("11:05"), "expected 11:05 (CEST), got: {out}");
        assert!(out.starts_with("⏸️"), "expected pause emoji prefix");
    }

    #[test]
    fn long_defer_renders_correctly() {
        // 1h = 3_600_000 ms; from 09:00 UTC + 1h = 10:00 UTC = 12:00 CEST
        std::env::set_var("LIBREFANG_TZ", "Europe/Rome");
        let err = "Model overloaded [rate_limit_defer_ms]=3600000";
        let out = build_rate_limit_notify_text(err, fixed_now()).expect("expected Some");
        assert!(out.contains("12:00"), "expected 12:00 (CEST), got: {out}");
    }

    #[test]
    fn env_var_overrides_default_tz() {
        // Tests share process env; clear template override that sibling
        // tests may have set when running in parallel.
        std::env::remove_var("LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE");
        std::env::set_var("LIBREFANG_TZ", "America/New_York");
        let err = "Rate limited [rate_limit_defer_ms]=300000";
        let out = build_rate_limit_notify_text(err, fixed_now()).expect("expected Some");
        // 09:05 UTC = 05:05 EDT (May = DST active)
        assert!(out.contains("05:05"), "expected EDT 05:05, got: {out}");
        std::env::set_var("LIBREFANG_TZ", "Europe/Rome");
    }

    #[test]
    fn env_var_overrides_template() {
        std::env::set_var("LIBREFANG_TZ", "Europe/Rome");
        std::env::set_var(
            "LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE",
            "Limite raggiunto. Reset alle {reset_time}.",
        );
        let err = "Rate limited [rate_limit_defer_ms]=300000";
        let out = build_rate_limit_notify_text(err, fixed_now()).expect("expected Some");
        assert_eq!(out, "Limite raggiunto. Reset alle 11:05.");
        std::env::remove_var("LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE");
    }

    #[test]
    fn template_without_placeholder_renders_verbatim() {
        std::env::set_var("LIBREFANG_TZ", "Europe/Rome");
        std::env::set_var("LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE", "Riprovo dopo.");
        let err = "Rate limited [rate_limit_defer_ms]=300000";
        let out = build_rate_limit_notify_text(err, fixed_now()).expect("expected Some");
        assert_eq!(out, "Riprovo dopo.");
        std::env::remove_var("LIBREFANG_RATE_LIMIT_NOTIFY_TEMPLATE");
    }
}
