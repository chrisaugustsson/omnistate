//! `omni wait`: blocks until a change set reaches a status, so an agent can
//! hand work to the human and carry on once it is decided. It only reads the
//! change set, again and again, and ends with a code a script can branch on:
//! 0 when the status is reached, 2 when `--timeout` ran out and 3 when the
//! status can no longer be reached.

use crate::args::Args;
use crate::changesets::{changeset_id, changesets_path, project_id, summary};
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use crate::thread_wait;
use std::time::{Duration, Instant};

const USAGE: &str = "omni wait --changeset <id> --until approved|rejected|merged|needs_rebase [--timeout <seconds>]";
const TIMED_OUT: u8 = 2;
const UNREACHABLE: u8 = 3;
/// How long to wait between two reads of the change set.
const POLL: Duration = Duration::from_millis(500);

/// The statuses that `--until` takes.
const TARGETS: [&str; 4] = ["approved", "rejected", "merged", "needs_rebase"];

/// Whether `status` meets the wait for `target`. The server merges an approved
/// change set at once or moves it to `needs_rebase`, so it never rests in
/// `approved`: a change set carries the approval in all three statuses.
fn is_met(target: &str, status: &str) -> bool {
    match target {
        "approved" => matches!(status, "approved" | "needs_rebase" | "merged"),
        _ => status == target,
    }
}

/// Whether `target` can still be reached from `status`: nothing follows an
/// abandoned change set, and a merged one is final.
fn is_lost(target: &str, status: &str) -> bool {
    status == "abandoned" || (status == "merged" && matches!(target, "rejected" | "needs_rebase"))
}

fn timeout(text: &str) -> Result<Duration> {
    match text.trim().parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Ok(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
        }
        _ => bail!("--timeout {text:?} is not a number of seconds: {USAGE}"),
    }
}

/// `omni wait --changeset <id> --until <status> [--timeout <seconds>]`
pub fn run(args: &Args) -> Result<()> {
    if let Some(thread) = args.thread.as_deref() {
        return thread_wait::run(args, thread);
    }
    if args.posts.is_some() || args.concluded {
        bail!(
            "--posts and --concluded wait on a thread; add --thread <id>: {}",
            thread_wait::USAGE
        );
    }
    let Some(changeset) = args.changeset.as_deref() else {
        bail!("this command needs --changeset <id>: {USAGE}");
    };
    let changeset = changeset_id(changeset)?;
    let Some(target) = args.until.as_deref() else {
        bail!("this command needs --until <status>: {USAGE}");
    };
    if !TARGETS.contains(&target) {
        bail!("--until {target:?} is not one of approved, rejected, merged, needs_rebase");
    }
    let limit = args.timeout.as_deref().map(timeout).transpose()?;

    let started = Instant::now();
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = format!("{}/{changeset}", changesets_path(&agent, &project));
    loop {
        let response = agent.request("GET", &path, None)?;
        let text = response.text();
        let body = Value::parse(&text).map_err(|_| {
            CliError::new(
                "the server's answer about the change set is not readable; is it an OmniState API?",
            )
        })?;
        let shown = body.get("changeset").unwrap_or(&body);
        let status = shown.str_at("status").unwrap_or("?");
        if is_met(target, status) {
            if args.json {
                say(text.trim_end());
            } else {
                say(&summary(shown));
            }
            return Ok(());
        }
        if is_lost(target, status) {
            return Err(CliError::with_code(
                UNREACHABLE,
                format!("change set {changeset} is {status}, so it can no longer become {target}"),
            ));
        }
        let remaining = match limit {
            Some(limit) => limit.saturating_sub(started.elapsed()),
            None => POLL,
        };
        if remaining.is_zero() {
            return Err(CliError::with_code(
                TIMED_OUT,
                format!(
                    "timed out waiting for change set {changeset} to become {target}; it is {status}"
                ),
            ));
        }
        std::thread::sleep(remaining.min(POLL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_is_met_by_every_status_that_carries_the_approval() {
        for status in ["approved", "needs_rebase", "merged"] {
            assert!(is_met("approved", status));
        }
        for status in ["open", "in_review", "rejected", "abandoned"] {
            assert!(!is_met("approved", status));
        }
        assert!(is_met("merged", "merged"));
        assert!(!is_met("merged", "needs_rebase"));
    }

    #[test]
    fn abandoned_and_merged_change_sets_lose_the_waits_that_cannot_follow() {
        for target in TARGETS {
            assert!(is_lost(target, "abandoned"));
        }
        assert!(is_lost("rejected", "merged"));
        assert!(is_lost("needs_rebase", "merged"));
        assert!(!is_lost("approved", "merged"));
        assert!(!is_lost("rejected", "needs_rebase"));
    }
}
