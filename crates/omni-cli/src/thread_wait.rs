//! `omni wait --thread <id> --posts <n>` and `--concluded`: blocks until a
//! thread has enough posts that this agent may read, or until it has a
//! conclusion, so a workflow can pause until the others have spoken. It only
//! reads the thread, again and again, and ends as `omni wait --changeset`
//! does: 0 when the condition is met and 2 when `--timeout` ran out.
//!
//! The thread read is the agent's own, so a sealed thread counts no post of
//! another agent until this agent has posted itself.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id};
use crate::commands::{agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use std::time::{Duration, Instant};

pub const USAGE: &str = "omni wait --thread <id> --posts <n> [--concluded] [--timeout <seconds>]";
const TIMED_OUT: u8 = 2;
/// How long to wait between two reads of the thread.
const POLL: Duration = Duration::from_millis(500);

/// What the wait is for: at least this many readable posts, and a conclusion
/// when `concluded`.
#[derive(Debug, PartialEq)]
struct Condition {
    posts: Option<usize>,
    concluded: bool,
}

impl Condition {
    fn new(args: &Args) -> Result<Condition> {
        if args.until.is_some() {
            bail!(
                "--until is for change sets; a thread is waited on with --posts <n> or --concluded: {USAGE}"
            );
        }
        let posts = args
            .posts
            .as_deref()
            .map(|text| match text.parse::<usize>() {
                Ok(count) if count >= 1 => Ok(count),
                _ => Err(CliError::new(format!(
                    "--posts {text:?} is not a whole number of at least 1: {USAGE}"
                ))),
            })
            .transpose()?;
        if posts.is_none() && !args.concluded {
            bail!("this command needs --posts <n> or --concluded: {USAGE}");
        }
        Ok(Condition {
            posts,
            concluded: args.concluded,
        })
    }

    /// Whether the thread, as the agent reads it, meets the condition.
    fn met_by(&self, thread: &Value) -> bool {
        let posts = thread
            .get("posts")
            .and_then(Value::as_array)
            .map_or(0, <[Value]>::len);
        self.posts.is_none_or(|wanted| posts >= wanted)
            && (!self.concluded || thread.str_at("conclusion").is_some())
    }

    /// The condition as the user wrote it, for a message.
    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(posts) = self.posts {
            parts.push(format!("{posts} posts"));
        }
        if self.concluded {
            parts.push("a conclusion".to_string());
        }
        parts.join(" and ")
    }
}

fn timeout(text: &str) -> Result<Duration> {
    match text.trim().parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Ok(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
        }
        _ => bail!("--timeout {text:?} is not a number of seconds: {USAGE}"),
    }
}

/// `omni wait --thread <id> --posts <n> [--concluded] [--timeout <seconds>]`
pub fn run(args: &Args, thread: &str) -> Result<()> {
    if args.changeset.is_some() {
        bail!("--thread and --changeset cannot go together; wait for one of them: {USAGE}");
    }
    if !is_uuid(thread) {
        bail!("{thread:?} is not a thread ID: {USAGE}");
    }
    let thread = thread.to_ascii_lowercase();
    let wanted = Condition::new(args)?;
    let limit = args.timeout.as_deref().map(timeout).transpose()?;

    let started = Instant::now();
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = format!(
        "/api/v1/w/{}/projects/{project}/threads/{thread}",
        agent.workspace_id
    );
    let not_found = format!("not_found: no thread {thread} in this project");
    loop {
        let response = agent.request_or_not_found("GET", &path, None, &not_found)?;
        let text = response.text();
        let body = Value::parse(&text).map_err(|_| {
            CliError::new(
                "the server's answer about the thread is not readable; is it an OmniState API?",
            )
        })?;
        if wanted.met_by(body.get("thread").unwrap_or(&body)) {
            if args.json {
                say(text.trim_end());
            } else {
                say(&format!("thread {thread}  has {}", wanted.describe()));
            }
            return Ok(());
        }
        let remaining = match limit {
            Some(limit) => limit.saturating_sub(started.elapsed()),
            None => POLL,
        };
        if remaining.is_zero() {
            return Err(CliError::with_code(
                TIMED_OUT,
                format!(
                    "timed out waiting for {} on thread {thread}",
                    wanted.describe()
                ),
            ));
        }
        std::thread::sleep(remaining.min(POLL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::parse;

    fn condition(args: &[&str]) -> Result<Condition> {
        Condition::new(&parse(args.iter().map(|a| a.to_string())).unwrap())
    }

    fn thread(posts: usize, conclusion: Option<&str>) -> Value {
        let posts: Vec<String> = (0..posts).map(|_| "{}".to_string()).collect();
        let conclusion = conclusion.map_or("null".to_string(), |c| format!("{c:?}"));
        Value::parse(&format!(
            "{{\"posts\":[{}],\"conclusion\":{conclusion}}}",
            posts.join(",")
        ))
        .unwrap()
    }

    #[test]
    fn a_thread_meets_the_posts_it_has_and_the_conclusion_it_was_given() {
        let posts = condition(&["wait", "--posts", "2"]).unwrap();
        assert!(!posts.met_by(&thread(1, None)));
        assert!(posts.met_by(&thread(2, None)));
        assert!(posts.met_by(&thread(3, Some("done"))));
        let concluded = condition(&["wait", "--concluded"]).unwrap();
        assert!(!concluded.met_by(&thread(5, None)));
        assert!(concluded.met_by(&thread(0, Some("done"))));
        let both = condition(&["wait", "--posts", "2", "--concluded"]).unwrap();
        assert!(!both.met_by(&thread(2, None)));
        assert!(!both.met_by(&thread(1, Some("done"))));
        assert!(both.met_by(&thread(2, Some("done"))));
    }
}
