//! `omni wait --thread <id> --until <condition>`: blocks until others have
//! posted in a thread, its phase has changed or the human has decided, so a
//! workflow can pause at the right moment. It only reads the thread, again and
//! again, and ends as `omni wait --changeset` does: 0 when the condition is
//! met, 2 when `--timeout` ran out and 3 when it can no longer be met.
//!
//! The phase a `posts:` or `roles:` wait keeps to is the thread's last phase at
//! the first read; it stays that one when a later phase starts. The thread's
//! `posted_by` tells who has posted in a phase, also in a sealed one, and never
//! what.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const USAGE: &str = "omni wait --thread <id> --until posts:<n>|roles:<pattern>[,<pattern>...]|phase-change|decision:approved|decision:rejected [--timeout <seconds>]";
const TIMED_OUT: u8 = 2;
const UNREACHABLE: u8 = 3;
/// How long to wait between two reads of the thread.
const POLL: Duration = Duration::from_millis(500);

/// What `--until` waits for.
#[derive(Debug, PartialEq)]
enum Condition {
    /// At least this many different agent identities posted in the phase.
    Posts(usize),
    /// Every pattern matches the role of an identity that posted in the phase.
    Roles(Vec<String>),
    PhaseChange,
    /// The thread's decision has this status.
    Decision(&'static str),
}

fn condition(text: &str) -> Result<Condition> {
    let bad = |why: &str| CliError::new(format!("--until {text:?} {why}: {USAGE}"));
    if text == "phase-change" {
        return Ok(Condition::PhaseChange);
    }
    if let Some(count) = text.strip_prefix("posts:") {
        return match count.parse::<usize>() {
            Ok(count) if count >= 1 => Ok(Condition::Posts(count)),
            _ => Err(bad("needs a whole number of at least 1 after posts:")),
        };
    }
    if let Some(patterns) = text.strip_prefix("roles:") {
        let patterns: Vec<String> = patterns.split(',').map(|p| p.trim().to_string()).collect();
        if patterns.iter().any(String::is_empty) {
            return Err(bad(
                "needs a role pattern after roles:, or several split by commas",
            ));
        }
        return Ok(Condition::Roles(patterns));
    }
    match text {
        "decision:approved" => Ok(Condition::Decision("approved")),
        "decision:rejected" => Ok(Condition::Decision("rejected")),
        _ => Err(bad("is not a condition for a thread")),
    }
}

/// Whether `role` matches `pattern` as a whole, `*` standing for any run of
/// characters (also none).
fn matches(pattern: &str, role: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let role: Vec<char> = role.chars().collect();
    let (mut p, mut r) = (0, 0);
    // Where the last `*` was, and how much of the role it has taken so far.
    let mut star: Option<(usize, usize)> = None;
    while r < role.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, r));
            p += 1;
        } else if p < pattern.len() && pattern[p] == role[r] {
            p += 1;
            r += 1;
        } else if let Some((at, taken)) = star {
            p = at + 1;
            r = taken + 1;
            star = Some((at, taken + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

fn timeout(text: &str) -> Result<Duration> {
    match text.trim().parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Ok(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
        }
        _ => bail!("--timeout {text:?} is not a number of seconds: {USAGE}"),
    }
}

/// The parts of a read thread that a wait looks at.
struct View {
    /// The number of the last phase.
    last: u64,
    /// Who posted in each phase: (number, (run, role) of each identity).
    phases: Vec<(u64, Vec<(String, String)>)>,
    /// The status of the thread's decision, if it has one.
    decision: Option<String>,
}

impl View {
    fn read(thread: &Value) -> Option<View> {
        let phases: Vec<(u64, Vec<(String, String)>)> = thread
            .get("phases")?
            .as_array()?
            .iter()
            .map(|phase| {
                let number = match phase.get("number") {
                    Some(Value::Number(n)) => n.parse().ok()?,
                    _ => return None,
                };
                let posted_by = phase
                    .get("posted_by")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                    .iter()
                    .map(|who| {
                        (
                            who.str_at("run").unwrap_or_default().to_string(),
                            who.str_at("role").unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                Some((number, posted_by))
            })
            .collect::<Option<_>>()?;
        let last = phases.iter().map(|(number, _)| *number).max()?;
        let decision = thread
            .get("decision")
            .and_then(|decision| decision.str_at("status"))
            .map(str::to_string);
        Some(View {
            last,
            phases,
            decision,
        })
    }

    /// The identities that posted in phase `number`, each once.
    fn posters(&self, number: u64) -> Vec<&(String, String)> {
        let mut seen = HashSet::new();
        self.phases
            .iter()
            .filter(|(n, _)| *n == number)
            .flat_map(|(_, posted_by)| posted_by)
            .filter(|who| seen.insert(*who))
            .collect()
    }
}

enum Verdict {
    Met,
    Pending,
    /// It can no longer be met, and why.
    Lost(String),
}

/// How the thread stands against the condition, where `phase` is the phase the
/// wait keeps to.
fn judge(condition: &Condition, phase: u64, view: &View) -> Verdict {
    let moved_on = || {
        Verdict::Lost(format!(
            "phase {} of the thread started before the condition was met in phase {phase}, so it can no longer be met",
            view.last
        ))
    };
    match condition {
        Condition::Posts(count) => {
            if view.posters(phase).len() >= *count {
                Verdict::Met
            } else if view.last > phase {
                moved_on()
            } else {
                Verdict::Pending
            }
        }
        Condition::Roles(patterns) => {
            let posters = view.posters(phase);
            let all = patterns
                .iter()
                .all(|pattern| posters.iter().any(|(_, role)| matches(pattern, role)));
            if all {
                Verdict::Met
            } else if view.last > phase {
                moved_on()
            } else {
                Verdict::Pending
            }
        }
        Condition::PhaseChange => {
            if view.last > phase {
                Verdict::Met
            } else {
                Verdict::Pending
            }
        }
        Condition::Decision(target) => match view.decision.as_deref() {
            Some(status) if status == *target => Verdict::Met,
            Some(status @ ("approved" | "rejected")) => Verdict::Lost(format!(
                "the thread's decision is {status}, so it can no longer become {target}"
            )),
            _ => Verdict::Pending,
        },
    }
}

/// `omni wait --thread <id> --until <condition> [--timeout <seconds>]`
pub fn run(args: &Args, thread: &str) -> Result<()> {
    if args.changeset.is_some() {
        bail!("--thread and --changeset cannot go together; wait for one of them: {USAGE}");
    }
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {USAGE}");
    }
    let thread = thread.to_ascii_lowercase();
    let Some(until) = args.until.as_deref() else {
        bail!("this command needs --until <condition>: {USAGE}");
    };
    let wanted = condition(until)?;
    let limit = args.timeout.as_deref().map(timeout).transpose()?;

    let started = Instant::now();
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = format!(
        "/api/v1/w/{}/projects/{project}/threads/{thread}",
        agent.workspace_id
    );
    let not_found = format!("not_found: no thread {thread} in this project");
    let mut phase = None;
    loop {
        let response = agent.request_or_not_found("GET", &path, None, &not_found)?;
        let text = response.text();
        let unreadable = || {
            CliError::new(
                "the server's answer about the thread is not readable; is it an OmniState API?",
            )
        };
        let body = Value::parse(&text).map_err(|_| unreadable())?;
        let view = View::read(body.get("thread").unwrap_or(&body)).ok_or_else(unreadable)?;
        let phase = *phase.get_or_insert(view.last);
        match judge(&wanted, phase, &view) {
            Verdict::Met => {
                if args.json {
                    say(text.trim_end());
                } else {
                    say(&format!("thread {thread}  {until} is met"));
                }
                return Ok(());
            }
            Verdict::Lost(why) => return Err(CliError::with_code(UNREACHABLE, why)),
            Verdict::Pending => {}
        }
        let remaining = match limit {
            Some(limit) => limit.saturating_sub(started.elapsed()),
            None => POLL,
        };
        if remaining.is_zero() {
            return Err(CliError::with_code(
                TIMED_OUT,
                format!("timed out waiting for {until} on thread {thread}"),
            ));
        }
        std::thread::sleep(remaining.min(POLL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(phases: &[(u64, &[&str])], decision: Option<&str>) -> View {
        View {
            last: phases.iter().map(|p| p.0).max().unwrap(),
            phases: phases
                .iter()
                .map(|(number, who)| {
                    let who = who
                        .iter()
                        .map(|w| {
                            let (run, role) = w.split_once('/').unwrap();
                            (run.to_string(), role.to_string())
                        })
                        .collect();
                    (*number, who)
                })
                .collect(),
            decision: decision.map(str::to_string),
        }
    }

    #[test]
    fn conditions_are_parsed_and_the_rest_refused() {
        assert_eq!(condition("posts:3").unwrap(), Condition::Posts(3));
        assert_eq!(condition("phase-change").unwrap(), Condition::PhaseChange);
        assert_eq!(
            condition("decision:rejected").unwrap(),
            Condition::Decision("rejected")
        );
        assert_eq!(
            condition("roles:critic-*, scout").unwrap(),
            Condition::Roles(vec!["critic-*".to_string(), "scout".to_string()])
        );
        for text in [
            "posts:",
            "posts:0",
            "posts:-2",
            "posts:abc",
            "roles:",
            "roles:a,,b",
            "decision:maybe",
            "approved",
            "",
        ] {
            assert!(condition(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_pattern_matches_the_whole_role_and_a_star_any_run() {
        for (pattern, role) in [
            ("critic-a", "critic-a"),
            ("critic-*", "critic-a"),
            ("*-a", "critic-a"),
            ("*", "anything"),
            ("*", ""),
            ("c*a", "critic-a"),
            ("*c*c*", "critic-c"),
            ("a**b", "ab"),
        ] {
            assert!(matches(pattern, role), "{pattern} {role}");
        }
        for (pattern, role) in [
            ("critic", "critic-a"),
            ("critic-", "critic-a"),
            ("ritic-a", "critic-a"),
            ("*-b", "critic-a"),
            ("a*b", "ac"),
            ("", "a"),
        ] {
            assert!(!matches(pattern, role), "{pattern} {role}");
        }
    }

    #[test]
    fn posts_count_identities_of_the_waited_phase_only() {
        let v = view(&[(1, &["r1/a", "r1/b", "r1/c"]), (2, &["r1/a"])], None);
        assert!(matches!(judge(&Condition::Posts(1), 2, &v), Verdict::Met));
        assert!(matches!(judge(&Condition::Posts(3), 1, &v), Verdict::Met));
        assert!(matches!(
            judge(&Condition::Posts(2), 2, &v),
            Verdict::Pending
        ));
        assert!(matches!(
            judge(&Condition::Posts(4), 1, &v),
            Verdict::Lost(_)
        ));
    }

    #[test]
    fn a_decision_of_the_other_final_status_is_lost() {
        let v = view(&[(1, &[])], Some("rejected"));
        assert!(matches!(
            judge(&Condition::Decision("rejected"), 1, &v),
            Verdict::Met
        ));
        assert!(matches!(
            judge(&Condition::Decision("approved"), 1, &v),
            Verdict::Lost(_)
        ));
        let v = view(&[(1, &[])], Some("proposed"));
        assert!(matches!(
            judge(&Condition::Decision("approved"), 1, &v),
            Verdict::Pending
        ));
    }
}
