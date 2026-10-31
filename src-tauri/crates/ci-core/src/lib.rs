//! Shared CI evidence decisions and GitHub Actions collection without Tauri.
//! Credential acquisition, database writes, process lifetime, and approvals remain with the host.
//! Observations are not permissions.

use chrono::{DateTime, Duration, Utc};

pub mod github;
pub use github::{GithubRunsResponse, GithubRun, GithubPullRef, github_client, github_repository_runs, fetch_json, validate_repo};

/// Map raw GitHub Actions status/conclusion to an observed CI state.
pub fn ci_state(status: &str, conclusion: Option<&str>) -> &'static str {
    match status {
        "in_progress" => "RUNNING",
        "queued" | "requested" | "pending" | "waiting" => "QUEUED",
        "completed" => match conclusion {
            Some("success") => "SUCCESS",
            Some("failure" | "timed_out" | "startup_failure") => "FAILURE",
            Some(_) => "OTHER_TERMINAL",
            None => "UNKNOWN",
        },
        _ => "UNKNOWN",
    }
}

/// Unknown producer naming schemes must not be guessed from workflow names.
pub fn ci_kind(path: Option<&str>) -> &'static str {
    match path.and_then(|p| p.strip_prefix(".github/workflows/")) {
        Some("ci.yml" | "ci.yaml") => "general",
        Some("ci-integration.yml" | "ci-integration.yaml") => "integration",
        Some(_) => "other",
        None => "unknown",
    }
}

/// A known integration run title may claim candidate PR/head identity.
/// It does not verify its pinned base or the current GitHub PR head.
pub fn integration_candidate(path: Option<&str>, title: &str) -> Option<(i64, String)> {
    if ci_kind(path) != "integration" { return None; }
    let value = title.strip_prefix("Integration PR #")?;
    let (number, head) = value.split_once(" at ")?;
    if number.starts_with('0') || !number.bytes().all(|b| b.is_ascii_digit()) { return None; }
    let number = number.parse::<i64>().ok().filter(|v| *v > 0)?;
    if head.len() != 40 || !head.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    Some((number, head.to_ascii_lowercase()))
}

/// Cached success is never enough: both the repository poll and individual
/// run observation must be recent, enabled, and on the latest known attempt.
pub fn freshness(
    now: DateTime<Utc>,
    enabled: bool,
    last_success: Option<&str>,
    last_error: Option<&str>,
    last_seen: &str,
    limit_seconds: i64,
    latest_attempt_matches: bool,
) -> &'static str {
    if !enabled { return "UNKNOWN"; }
    if last_error.is_some() { return "POLL_ERROR"; }
    let parse = |s: &str| DateTime::parse_from_rfc3339(s).ok().map(|v| v.with_timezone(&Utc));
    let Some(success) = last_success.and_then(parse) else { return "UNKNOWN"; };
    let Some(seen) = parse(last_seen) else { return "UNKNOWN"; };
    if !latest_attempt_matches || success > now + Duration::seconds(10)
        || seen > now + Duration::seconds(10)
        || (now - success).num_seconds() > limit_seconds
        || (now - seen).num_seconds() > limit_seconds {
        return "STALE";
    }
    "FRESH"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_actions_states_do_not_promote_non_success_to_green() {
        for status in ["queued", "requested", "pending", "waiting"] {
            assert_eq!(ci_state(status, None), "QUEUED");
        }
        assert_eq!(ci_state("in_progress", None), "RUNNING");
        assert_eq!(ci_state("completed", Some("success")), "SUCCESS");
        for failure in ["failure", "timed_out", "startup_failure"] {
            assert_eq!(ci_state("completed", Some(failure)), "FAILURE");
        }
        for terminal in ["cancelled", "skipped", "neutral", "action_required"] {
            assert_eq!(ci_state("completed", Some(terminal)), "OTHER_TERMINAL");
        }
        assert_eq!(ci_state("completed", None), "UNKNOWN");
        assert_eq!(ci_state("unexpected", Some("success")), "UNKNOWN");
    }

    #[test]
    fn integration_candidate_is_observation_only_and_requires_exact_format() {
        let head = "ABCDEF0123456789abcdef0123456789abcdef01";
        assert_eq!(ci_kind(Some(".github/workflows/ci.yml")), "general");
        assert_eq!(ci_kind(Some(".github/workflows/ci-integration.yaml")), "integration");
        assert_eq!(ci_kind(Some(".github/workflows/custom-ci.yml")), "other");
        assert_eq!(ci_kind(None), "unknown");
        assert_eq!(integration_candidate(Some(".github/workflows/ci-integration.yml"),
            &format!("Integration PR #123 at {head}")), Some((123, head.to_lowercase())));
        for title in ["Integration PR #0 at a", "Integration PR #0123 at a", "Integration PR #123 at invalid",
            "Integration PR #123 at abcdef0123456789abcdef0123456789abcdef01 trailing"] {
            assert_eq!(integration_candidate(Some(".github/workflows/ci-integration.yml"), title), None);
        }
        assert_eq!(integration_candidate(Some(".github/workflows/ci.yml"),
            &format!("Integration PR #123 at {head}")), None);
    }

    #[test]
    fn freshness_fail_closed_for_stale_attempt_poll_and_missing_timestamp() {
        let now = Utc::now();
        let current = now.to_rfc3339();
        let old = (now - Duration::seconds(400)).to_rfc3339();
        let future = (now + Duration::seconds(20)).to_rfc3339();
        assert_eq!(freshness(now, true, Some(&current), None, &current, 180, true), "FRESH");
        assert_eq!(freshness(now, false, Some(&current), None, &current, 180, true), "UNKNOWN");
        assert_eq!(freshness(now, true, Some(&current), Some("HTTP 401"), &current, 180, true), "POLL_ERROR");
        assert_eq!(freshness(now, true, None, None, &current, 180, true), "UNKNOWN");
        assert_eq!(freshness(now, true, Some(&current), None, "invalid", 180, true), "UNKNOWN");
        assert_eq!(freshness(now, true, Some(&old), None, &current, 180, true), "STALE");
        assert_eq!(freshness(now, true, Some(&current), None, &old, 180, true), "STALE");
        assert_eq!(freshness(now, true, Some(&current), None, &current, 180, false), "STALE");
        assert_eq!(freshness(now, true, Some(&future), None, &current, 180, true), "STALE");
    }
}
