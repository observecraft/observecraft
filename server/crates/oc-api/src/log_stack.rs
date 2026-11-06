//! Stack traces in plain-text logs: a multi-line message (the collector joined the lines, see the agent chart's
//! `logs.multiline`) whose lines include stack frames gets Datadog's error attributes, unless the log already has
//! them: `error.stack` (from the exception's first line to the end), `error.kind` and `error.message` (the
//! exception's type and text). Error Tracking groups error logs by them (`routes/error_tracking.rs`), and the log
//! side panel shows the stack.
//!
//! Recognised: Java / Kotlin / Scala / .NET (`at …(…)`, `Caused by:`), Python (`Traceback`, `File "…", line N`),
//! Node.js (`at … (file:line:col)`), Go (`goroutine N [running]:`, `panic:`) and Ruby (`from file:N:in`).

use std::sync::LazyLock;

use regex::Regex;

static FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:\s+at \S.*[(:]\S*\d|\s+at \S+\(|\s+\.\.\. \d+ (?:more|common frames omitted)",
        r#"|\s+File ".+", line \d+|Traceback \(most recent call last\):"#,
        r"|goroutine \d+ \[|\s+\S+\.go:\d+|\s*from \S+:\d+:in )"
    ))
    .expect("frame regex")
});

/// An exception's first line (Java, .NET, Node.js) or last line (Python): `java.lang.IllegalStateException: text`,
/// `ValueError: text`, `TypeError: text`, `panic: text`.
static EXCEPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:Exception in thread .+? )?(?P<kind>[A-Za-z_$][\w.$]*(?:Exception|Error|Throwable|Panic|Fault)|panic)(?::\s*(?P<msg>.*))?$").expect("exception regex"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackTrace {
    pub kind: String,
    pub message: String,
    pub stack: String,
}

/// The stack trace of a multi-line message, if it has stack frames.
pub fn extract(message: &str) -> Option<StackTrace> {
    if !message.contains('\n') {
        return None;
    }
    let lines: Vec<&str> = message.lines().collect();
    let first_frame = lines.iter().position(|l| FRAME.is_match(l))?;
    let python = lines.iter().any(|l| l.starts_with("Traceback (most recent call last):"));
    // Python names the exception after the frames; the others before them.
    let exception = if python {
        lines.iter().enumerate().rev().find(|(_, l)| !l.starts_with(char::is_whitespace) && EXCEPTION.is_match(l))
    } else {
        lines[..=first_frame].iter().enumerate().rev().find(|(_, l)| EXCEPTION.is_match(l.trim()))
    };
    let start = if python {
        lines.iter().position(|l| l.starts_with("Traceback")).unwrap_or(first_frame)
    } else {
        exception.map(|(i, _)| i).unwrap_or(first_frame)
    };
    let (kind, msg) = match exception.and_then(|(_, l)| EXCEPTION.captures(l.trim())) {
        Some(c) => (c.name("kind").map_or("", |m| m.as_str()).to_string(), c.name("msg").map_or("", |m| m.as_str()).trim().to_string()),
        None => (String::new(), String::new()),
    };
    Some(StackTrace { kind, message: msg, stack: lines[start..].join("\n") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_with_its_cause() {
        let m = "2026-11-06 10:00:01 ERROR order-service failed to place order\n\
                 java.lang.IllegalStateException: inventory unavailable\n\
                 \tat com.example.orders.OrderService.place(OrderService.java:42)\n\
                 Caused by: java.net.SocketTimeoutException: Read timed out\n\
                 \tat java.base/java.net.SocketInputStream.read(SocketInputStream.java:168)\n\
                 \t... 12 more";
        let s = extract(m).unwrap();
        assert_eq!(s.kind, "java.lang.IllegalStateException");
        assert_eq!(s.message, "inventory unavailable");
        assert!(s.stack.starts_with("java.lang.IllegalStateException: inventory unavailable\n\tat com.example"));
        assert!(s.stack.ends_with("... 12 more"));
    }

    #[test]
    fn python_names_the_exception_last() {
        let m = "Traceback (most recent call last):\n  File \"/app/billing.py\", line 10, in charge\n    raise ValueError(\"card declined\")\nValueError: card declined";
        let s = extract(m).unwrap();
        assert_eq!((s.kind.as_str(), s.message.as_str()), ("ValueError", "card declined"));
        assert_eq!(s.stack, m);
    }

    #[test]
    fn node_go_and_ruby() {
        let node = "TypeError: Cannot read properties of undefined (reading 'id')\n    at handler (/app/src/cart.js:12:18)\n    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)";
        assert_eq!(extract(node).unwrap().kind, "TypeError");
        let go = "panic: runtime error: index out of range [3] with length 2\n\ngoroutine 1 [running]:\nmain.main()\n\t/app/main.go:9 +0x1d";
        let s = extract(go).unwrap();
        assert_eq!((s.kind.as_str(), s.message.as_str()), ("panic", "runtime error: index out of range [3] with length 2"));
        let ruby = "NoMethodError: undefined method `total' for nil\n  from /app/cart.rb:12:in `checkout'";
        assert_eq!(extract(ruby).unwrap().kind, "NoMethodError");
    }

    #[test]
    fn plain_multi_line_messages_are_left_alone() {
        assert_eq!(extract("one line"), None);
        assert_eq!(extract("SELECT *\n  FROM orders\n  WHERE id = 1"), None);
        assert_eq!(extract("config:\n  port: 8080\n  host: example.com"), None);
    }
}
