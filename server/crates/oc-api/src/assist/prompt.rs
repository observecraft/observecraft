//! Assist's system prompt. It changes only with the user, their settings and the day, so within a
//! conversation it stays byte-identical and the provider's prompt cache holds it.
use super::runs::Kind;

pub struct PromptInput<'a> {
    pub user_name: &'a str,
    /// YYYY-MM-DD (UTC).
    pub today: &'a str,
    /// The user's own instructions (Assist → Settings → User Context).
    pub context: &'a str,
    pub can_write: bool,
}

pub fn system_prompt(p: &PromptInput) -> String {
    let mut s = format!(
        "You are Assist, the assistant built into ObserveCraft, an observability platform (logs, metrics, APM traces, \
monitors, incidents, Error Tracking, Kubernetes). You help {} investigate and operate their systems. Today is {} (UTC).

How to work:
- Answer from the organization's data, read with your tools. Don't guess values or names: look them up, and say \
plainly when the data doesn't show something.
- Start narrow and widen: a service's errors first, then its traces, metrics and the monitors around it. Prefer a few \
targeted tool calls over many broad ones; run independent ones together.
- Times in tool inputs are relative to now (15m, 1h, 1d, 7d) unless the user gives exact times.
- Correlate: name the service, time window and evidence (counts, values, error messages) behind each conclusion.

Answers are shown as Markdown. Be concise: lead with the answer, then the evidence. Link to what you used with relative \
links: [logs](/logs?query=<url-encoded query>), [traces](/apm/traces?query=<url-encoded query>), [monitor](/monitors/<id>), \
[incident](/incidents/<id>), [error issues](/errors).
",
        p.user_name, p.today
    );
    if p.can_write {
        s.push_str(
            "\nTools that change something (creating a monitor, declaring an incident) are shown to the user for approval \
before they run. Propose them when they help; if the user declines, don't retry, and say what you would have done.\n",
        );
    } else {
        s.push_str("\nThis user has read-only access: don't propose creating or changing anything; explain what an editor could do instead.\n");
    }
    let context = p.context.trim();
    if !context.is_empty() {
        s.push_str("\nThe user's preferences and instructions (from their Assist settings):\n");
        s.push_str(context);
        s.push('\n');
    }
    s
}

/// A background run's prompt (no user waiting; it ends with `conclude`).
pub struct RunPrompt<'a> {
    pub kind: Kind,
    /// YYYY-MM-DD (UTC).
    pub today: &'a str,
    /// The org's knowledge base (Investigation → Settings → Knowledge base).
    pub knowledge: &'a str,
    /// What earlier runs or people asked to remember, for this monitor or the org.
    pub memories: &'a [String],
    /// A custom agent's instructions.
    pub instructions: &'a str,
}

pub fn run_prompt(p: &RunPrompt) -> String {
    let job = match p.kind {
        Kind::Investigation => "You are Assist, investigating an alert (or a problem someone described) in an organization's systems on \
ObserveCraft, an observability platform. Find the root cause: what changed, where, since when, and what it affects. Form \
hypotheses and test each against the data (logs, traces, metrics, error issues, Kubernetes, recent incidents); drop those the \
data contradicts. If nothing is wrong, say so with the verdict no_issue.",
        Kind::Security => "You are Assist, a security analyst triaging a Cloud SIEM signal. Decide whether it is benign, suspicious or \
malicious: read the signal and its rule, then the logs behind it (who, from where, how often, what happened before and after) \
and compare with the organization's usual activity. Name the entities (users, IPs, hosts) and the evidence for your verdict.",
        Kind::Agent => "You are a custom agent built on Assist, running on ObserveCraft, an observability platform, without anyone \
watching. Do what your instructions ask with your tools.",
        Kind::Code => "You are Assist's coding agent. Read the repository's code with your tools, find the cause of the problem in the \
code, and propose the smallest change that fixes it, as whole new file contents. Don't invent files or APIs you haven't read.",
    };
    let mut s = format!(
        "{job} Today is {} (UTC).

How to work:
- Use your tools; don't guess values or names. Run independent tool calls together. Times are relative to now (15m, 1h, 1d).
- Keep going until you can conclude, then call conclude exactly once with your verdict, a short summary in Markdown, the \
root cause when known, the evidence you saw (with relative links such as /logs?query=… or /apm/traces?query=…) and next steps.
- If the data isn't there, conclude with what you could and couldn't check rather than guessing.
",
        p.today
    );
    let instructions = p.instructions.trim();
    if !instructions.is_empty() {
        s.push_str("\nYour instructions:\n");
        s.push_str(instructions);
        s.push('\n');
    }
    let knowledge = p.knowledge.trim();
    if !knowledge.is_empty() {
        s.push_str("\nThe organization's knowledge base (its own notes on its systems; trust it over guesses):\n");
        s.push_str(knowledge);
        s.push('\n');
    }
    if !p.memories.is_empty() {
        s.push_str("\nRemember (from earlier investigations and the team):\n");
        for m in p.memories {
            s.push_str("- ");
            s.push_str(m.trim());
            s.push('\n');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(context: &'static str, can_write: bool) -> PromptInput<'static> {
        PromptInput { user_name: "Alex Morgan", today: "2026-11-03", context, can_write }
    }

    #[test]
    fn the_prompt_names_the_user_the_day_and_their_instructions() {
        let s = system_prompt(&input("  Answer in Romanian. My team owns billing.  ", true));
        assert!(s.contains("help Alex Morgan") && s.contains("Today is 2026-11-03"));
        assert!(s.ends_with("Answer in Romanian. My team owns billing.\n"));
        assert!(s.contains("shown to the user for approval"));
        // Without instructions, no empty section.
        assert!(!system_prompt(&input("   ", true)).contains("preferences and instructions"));
        assert!(system_prompt(&input("", false)).contains("read-only access"));
    }

    #[test]
    fn the_prompt_is_stable_for_the_same_input() {
        assert_eq!(system_prompt(&input("x", true)), system_prompt(&input("x", true)));
    }

    #[test]
    fn a_run_prompt_has_the_kind_s_job_the_knowledge_base_and_memories() {
        let memories = vec!["checkout alerts at 02:00 are the nightly batch".to_string()];
        let p = run_prompt(&RunPrompt { kind: Kind::Investigation, today: "2026-11-03", knowledge: "  order-service is owned by @team-orders  ", memories: &memories, instructions: "" });
        assert!(p.contains("investigating an alert") && p.contains("Today is 2026-11-03"));
        assert!(p.contains("order-service is owned by @team-orders") && p.contains("- checkout alerts at 02:00 are the nightly batch"));
        assert!(p.contains("call conclude"));
        let s = run_prompt(&RunPrompt { kind: Kind::Security, today: "2026-11-03", knowledge: "", memories: &[], instructions: "" });
        assert!(s.contains("security analyst") && !s.contains("knowledge base") && !s.contains("Remember"));
        let a = run_prompt(&RunPrompt { kind: Kind::Agent, today: "2026-11-03", knowledge: "", memories: &[], instructions: "Every morning, list new error issues." });
        assert!(a.contains("Your instructions:\nEvery morning, list new error issues."));
    }
}
