//! On-Call escalation: the page state machine (pure functions, no I/O).
//!
//! A page is `triggered` until someone acknowledges or resolves it. While triggered, the
//! escalation worker notifies the current step of the page's escalation policy, then moves to the
//! next step after that step's delay; after the last step the policy starts over `repeat` more
//! times, then the page stays triggered without further notifications. Acknowledging stops the
//! escalation; reassigning puts the page back to `triggered` at the first step of the new policy.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OnCallTargetKind {
    /// One person.
    User,
    /// Whoever is on call on the schedule when the step runs.
    Schedule,
    /// Every member of the team.
    Team,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallTarget {
    pub kind: OnCallTargetKind,
    /// User id, schedule id or team handle.
    pub id: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnCallStep {
    pub targets: Vec<OnCallTarget>,
    /// Minutes to wait for an acknowledgement before the next step (1–1440).
    pub escalate_after_minutes: i32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OnCallPageStatus {
    Triggered,
    Acknowledged,
    Resolved,
}

impl OnCallPageStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "triggered" => Some(Self::Triggered),
            "acknowledged" => Some(Self::Acknowledged),
            "resolved" => Some(Self::Resolved),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAction {
    Acknowledge,
    Resolve,
    /// Back to triggered at the first step (of a new policy or team).
    Reassign,
}

/// The status after an action, or why it isn't possible.
pub fn transition(status: OnCallPageStatus, action: PageAction) -> Result<OnCallPageStatus, &'static str> {
    use OnCallPageStatus::*;
    match (status, action) {
        (Triggered, PageAction::Acknowledge) => Ok(Acknowledged),
        (Acknowledged, PageAction::Acknowledge) => Err("The page is already acknowledged"),
        (Resolved, _) => Err("The page is resolved"),
        (_, PageAction::Resolve) => Ok(Resolved),
        (_, PageAction::Reassign) => Ok(Triggered),
    }
}

/// Where a triggered page stands in its policy: the step notified next and the pass through the steps (0 = first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub step: usize,
    pub round: u32,
}

/// After notifying `at`'s step: the next position and the minutes to wait for it, or `None` when the policy is exhausted.
pub fn after_notifying(steps: &[OnCallStep], repeat: u32, at: Progress) -> Option<(Progress, i32)> {
    let wait = steps.get(at.step)?.escalate_after_minutes.max(1);
    if at.step + 1 < steps.len() {
        Some((Progress { step: at.step + 1, round: at.round }, wait))
    } else if at.round < repeat {
        Some((Progress { step: 0, round: at.round + 1 }, wait))
    } else {
        None
    }
}

pub fn steps_problem(steps: &[OnCallStep], repeat: i32, resolve_after_minutes: Option<i32>) -> Option<String> {
    if steps.is_empty() || steps.len() > 10 {
        return Some("An escalation policy has 1–10 steps".into());
    }
    for (i, s) in steps.iter().enumerate() {
        if s.targets.is_empty() || s.targets.len() > 20 {
            return Some(format!("Step {}: notify 1–20 users, schedules or teams", i + 1));
        }
        if !(1..=1440).contains(&s.escalate_after_minutes) {
            return Some(format!("Step {}: escalate after 1–1440 minutes", i + 1));
        }
    }
    if !(0..=10).contains(&repeat) {
        return Some("Repeat the policy 0–10 times".into());
    }
    if resolve_after_minutes.is_some_and(|m| !(1..=43_200).contains(&m)) {
        return Some("Resolve pages automatically after 1–43200 minutes".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use OnCallPageStatus::*;

    fn step(minutes: i32) -> OnCallStep {
        OnCallStep { targets: vec![OnCallTarget { kind: OnCallTargetKind::User, id: "u".into() }], escalate_after_minutes: minutes }
    }

    #[test]
    fn acknowledging_and_resolving() {
        assert_eq!(transition(Triggered, PageAction::Acknowledge), Ok(Acknowledged));
        assert!(transition(Acknowledged, PageAction::Acknowledge).is_err());
        assert_eq!(transition(Triggered, PageAction::Resolve), Ok(Resolved));
        assert_eq!(transition(Acknowledged, PageAction::Resolve), Ok(Resolved));
        assert!(transition(Resolved, PageAction::Resolve).is_err());
        assert!(transition(Resolved, PageAction::Acknowledge).is_err());
        assert_eq!(transition(Acknowledged, PageAction::Reassign), Ok(Triggered));
        assert!(transition(Resolved, PageAction::Reassign).is_err());
    }

    #[test]
    fn steps_run_in_order_with_their_delays() {
        let steps = [step(5), step(15), step(30)];
        let p0 = Progress { step: 0, round: 0 };
        assert_eq!(after_notifying(&steps, 0, p0), Some((Progress { step: 1, round: 0 }, 5)));
        assert_eq!(after_notifying(&steps, 0, Progress { step: 1, round: 0 }), Some((Progress { step: 2, round: 0 }, 15)));
        assert_eq!(after_notifying(&steps, 0, Progress { step: 2, round: 0 }), None);
    }

    #[test]
    fn the_policy_repeats_then_stops() {
        let steps = [step(10), step(20)];
        let mut at = Progress { step: 0, round: 0 };
        let mut notified = vec![];
        while let Some((next, _)) = {
            notified.push((at.step, at.round));
            after_notifying(&steps, 2, at)
        } {
            at = next;
        }
        assert_eq!(notified, vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)]);
        // The wait after the last step is that step's delay.
        assert_eq!(after_notifying(&steps, 2, Progress { step: 1, round: 0 }), Some((Progress { step: 0, round: 1 }, 20)));
    }

    #[test]
    fn out_of_range_positions_end_the_escalation() {
        assert_eq!(after_notifying(&[step(5)], 3, Progress { step: 4, round: 0 }), None);
        assert_eq!(after_notifying(&[], 3, Progress { step: 0, round: 0 }), None);
    }

    #[test]
    fn validation() {
        assert!(steps_problem(&[step(5)], 0, None).is_none());
        assert!(steps_problem(&[], 0, None).is_some());
        assert!(steps_problem(&[step(0)], 0, None).is_some());
        assert!(steps_problem(&[OnCallStep { targets: vec![], escalate_after_minutes: 5 }], 0, None).is_some());
        assert!(steps_problem(&[step(5)], 11, None).is_some());
        assert!(steps_problem(&[step(5)], 0, Some(0)).is_some());
        assert!(steps_problem(&[step(5)], 0, Some(60)).is_none());
        assert_eq!(OnCallPageStatus::parse("acknowledged"), Some(Acknowledged));
        assert_eq!(OnCallPageStatus::parse("paused"), None);
    }
}
