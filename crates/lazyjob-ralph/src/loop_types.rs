use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::{RalphError, Result};

const DISPATCH_CAP: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    JobDiscovery,
    CompanyResearch,
    ResumeTailor,
    CoverLetter,
    InterviewPrep,
}

impl LoopType {
    pub fn concurrency_limit(&self) -> usize {
        match self {
            LoopType::JobDiscovery => 1,
            LoopType::CompanyResearch => 2,
            LoopType::ResumeTailor => 3,
            LoopType::CoverLetter => 3,
            LoopType::InterviewPrep => 2,
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            LoopType::CoverLetter => 90,
            LoopType::ResumeTailor => 85,
            LoopType::InterviewPrep => 70,
            LoopType::CompanyResearch => 50,
            LoopType::JobDiscovery => 30,
        }
    }

    pub fn is_interactive(&self) -> bool {
        matches!(self, LoopType::InterviewPrep)
    }

    pub fn cli_subcommand(&self) -> &str {
        match self {
            LoopType::JobDiscovery => "job-discovery",
            LoopType::CompanyResearch => "company-research",
            LoopType::ResumeTailor => "resume-tailor",
            LoopType::CoverLetter => "cover-letter",
            LoopType::InterviewPrep => "interview-prep",
        }
    }
}

pub struct QueuedLoop {
    pub loop_type: LoopType,
    pub params: serde_json::Value,
    enqueued_at: Instant,
}

impl QueuedLoop {
    pub fn new(loop_type: LoopType, params: serde_json::Value) -> Self {
        Self {
            loop_type,
            params,
            enqueued_at: Instant::now(),
        }
    }
}

impl PartialEq for QueuedLoop {
    fn eq(&self, other: &Self) -> bool {
        self.loop_type.priority() == other.loop_type.priority()
            && self.enqueued_at == other.enqueued_at
    }
}

impl Eq for QueuedLoop {}

impl PartialOrd for QueuedLoop {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedLoop {
    fn cmp(&self, other: &Self) -> Ordering {
        let priority_ord = self.loop_type.priority().cmp(&other.loop_type.priority());
        if priority_ord != Ordering::Equal {
            return priority_ord;
        }
        other.enqueued_at.cmp(&self.enqueued_at)
    }
}

pub struct LoopDispatch {
    heap: BinaryHeap<QueuedLoop>,
    cap: usize,
}

impl LoopDispatch {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            cap: DISPATCH_CAP,
        }
    }

    pub fn enqueue(&mut self, loop_type: LoopType, params: serde_json::Value) -> Result<()> {
        if self.heap.len() >= self.cap {
            return Err(RalphError::QueueFull(self.cap));
        }
        self.heap.push(QueuedLoop::new(loop_type, params));
        Ok(())
    }

    pub fn drain_next(&mut self) -> Option<QueuedLoop> {
        self.heap.pop()
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

impl Default for LoopDispatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_loop_types_have_positive_priority() {
        let types = [
            LoopType::JobDiscovery,
            LoopType::CompanyResearch,
            LoopType::ResumeTailor,
            LoopType::CoverLetter,
            LoopType::InterviewPrep,
        ];
        for lt in types {
            assert!(lt.priority() > 0, "{:?} priority must be > 0", lt);
        }
    }

    #[test]
    fn cover_letter_higher_priority_than_discovery() {
        assert!(LoopType::CoverLetter.priority() > LoopType::JobDiscovery.priority());
    }

    #[test]
    fn resume_tailor_higher_priority_than_company_research() {
        assert!(LoopType::ResumeTailor.priority() > LoopType::CompanyResearch.priority());
    }

    #[test]
    fn job_discovery_concurrency_limit_is_1() {
        assert_eq!(LoopType::JobDiscovery.concurrency_limit(), 1);
    }

    #[test]
    fn all_concurrency_limits_positive() {
        let types = [
            LoopType::JobDiscovery,
            LoopType::CompanyResearch,
            LoopType::ResumeTailor,
            LoopType::CoverLetter,
            LoopType::InterviewPrep,
        ];
        for lt in types {
            assert!(lt.concurrency_limit() >= 1);
        }
    }

    #[test]
    fn interactive_only_for_interview_prep() {
        assert!(LoopType::InterviewPrep.is_interactive());
        assert!(!LoopType::JobDiscovery.is_interactive());
        assert!(!LoopType::CompanyResearch.is_interactive());
        assert!(!LoopType::ResumeTailor.is_interactive());
        assert!(!LoopType::CoverLetter.is_interactive());
    }

    #[test]
    fn cli_subcommands_are_kebab_case() {
        let types = [
            LoopType::JobDiscovery,
            LoopType::CompanyResearch,
            LoopType::ResumeTailor,
            LoopType::CoverLetter,
            LoopType::InterviewPrep,
        ];
        for lt in types {
            let sub = lt.cli_subcommand();
            assert!(!sub.is_empty(), "subcommand must not be empty for {:?}", lt);
            assert!(
                sub.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "subcommand must be kebab-case for {:?}: got '{}'",
                lt,
                sub
            );
        }
    }

    #[test]
    fn cli_subcommands_unique() {
        let types = [
            LoopType::JobDiscovery,
            LoopType::CompanyResearch,
            LoopType::ResumeTailor,
            LoopType::CoverLetter,
            LoopType::InterviewPrep,
        ];
        let subs: Vec<_> = types.iter().map(|lt| lt.cli_subcommand()).collect();
        let mut unique = subs.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(subs.len(), unique.len(), "all subcommands must be unique");
    }

    #[test]
    fn loop_dispatch_priority_ordering() {
        let mut dispatch = LoopDispatch::new();
        dispatch
            .enqueue(LoopType::JobDiscovery, json!(null))
            .unwrap();
        dispatch
            .enqueue(LoopType::CoverLetter, json!(null))
            .unwrap();
        dispatch
            .enqueue(LoopType::CompanyResearch, json!(null))
            .unwrap();

        let first = dispatch.drain_next().unwrap();
        let second = dispatch.drain_next().unwrap();
        let third = dispatch.drain_next().unwrap();

        assert_eq!(first.loop_type, LoopType::CoverLetter);
        assert_eq!(second.loop_type, LoopType::CompanyResearch);
        assert_eq!(third.loop_type, LoopType::JobDiscovery);
    }

    #[test]
    fn loop_dispatch_respects_cap() {
        let mut dispatch = LoopDispatch::new();
        for _ in 0..DISPATCH_CAP {
            dispatch
                .enqueue(LoopType::JobDiscovery, json!(null))
                .unwrap();
        }
        let result = dispatch.enqueue(LoopType::JobDiscovery, json!(null));
        assert!(matches!(result, Err(RalphError::QueueFull(20))));
    }

    #[test]
    fn loop_dispatch_drain_empty_returns_none() {
        let mut dispatch = LoopDispatch::new();
        assert!(dispatch.drain_next().is_none());
    }

    #[test]
    fn loop_dispatch_len_and_is_empty() {
        let mut dispatch = LoopDispatch::new();
        assert!(dispatch.is_empty());
        assert_eq!(dispatch.len(), 0);

        dispatch.enqueue(LoopType::ResumeTailor, json!({})).unwrap();
        assert!(!dispatch.is_empty());
        assert_eq!(dispatch.len(), 1);

        dispatch.drain_next();
        assert!(dispatch.is_empty());
    }

    #[test]
    fn queued_loop_same_priority_earlier_enqueue_wins() {
        let mut dispatch = LoopDispatch::new();
        dispatch
            .enqueue(LoopType::CoverLetter, json!({"seq": 1}))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        dispatch
            .enqueue(LoopType::CoverLetter, json!({"seq": 2}))
            .unwrap();

        let first = dispatch.drain_next().unwrap();
        assert_eq!(first.params["seq"], 1);
    }

    #[test]
    fn loop_type_serde_roundtrip() {
        let lt = LoopType::ResumeTailor;
        let s = serde_json::to_string(&lt).unwrap();
        let deserialized: LoopType = serde_json::from_str(&s).unwrap();
        assert_eq!(lt, deserialized);
    }
}
