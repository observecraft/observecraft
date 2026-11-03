//! Assist's system prompt. It changes only with the user, their settings and the day, so within a
//! conversation it stays byte-identical and the provider's prompt cache holds it.

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
}
