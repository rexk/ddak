use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOutputEvent {
    pub session_id: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDroppedEvent {
    pub session_id: String,
    pub dropped_count: usize,
    pub window_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FanoutEvent {
    Output(SessionOutputEvent),
    OutputDropped(OutputDroppedEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionActivitySummary {
    pub queued_events: usize,
    pub dropped_events: usize,
}

#[derive(Debug)]
pub struct LocalEventFanout {
    max_events_per_session: usize,
    queues: HashMap<String, VecDeque<FanoutEvent>>,
    dropped_counts: HashMap<String, usize>,
}

impl LocalEventFanout {
    pub fn new(max_events_per_session: usize) -> Self {
        Self {
            max_events_per_session,
            queues: HashMap::new(),
            dropped_counts: HashMap::new(),
        }
    }

    pub fn publish_output(&mut self, session_id: &str, data: impl Into<String>) {
        let queue = self.queues.entry(session_id.to_string()).or_default();
        let dropped = self
            .dropped_counts
            .entry(session_id.to_string())
            .or_insert(0);

        while queue.len() >= self.max_events_per_session {
            queue.pop_front();
            *dropped += 1;
        }

        if *dropped > 0 {
            queue.push_back(FanoutEvent::OutputDropped(OutputDroppedEvent {
                session_id: session_id.to_string(),
                dropped_count: *dropped,
                window_size: self.max_events_per_session,
            }));
            *dropped = 0;
        }

        queue.push_back(FanoutEvent::Output(SessionOutputEvent {
            session_id: session_id.to_string(),
            data: data.into(),
        }));
    }

    pub fn drain_session(&mut self, session_id: &str) -> Vec<FanoutEvent> {
        self.queues
            .entry(session_id.to_string())
            .or_default()
            .drain(..)
            .collect()
    }

    pub fn summary(&self, session_id: &str) -> SessionActivitySummary {
        SessionActivitySummary {
            queued_events: self.queues.get(session_id).map_or(0, |q| q.len()),
            dropped_events: self.dropped_counts.get(session_id).copied().unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FanoutEvent, LocalEventFanout};

    #[test]
    fn fanout_drains_output_for_active_session() {
        let mut fanout = LocalEventFanout::new(8);
        fanout.publish_output("sess-1", "line one");
        fanout.publish_output("sess-1", "line two");

        let events = fanout.drain_session("sess-1");
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], FanoutEvent::Output(_)));
        assert!(matches!(events[1], FanoutEvent::Output(_)));
    }

    #[test]
    fn overflow_emits_output_dropped_telemetry() {
        let mut fanout = LocalEventFanout::new(2);
        fanout.publish_output("sess-1", "a");
        fanout.publish_output("sess-1", "b");
        fanout.publish_output("sess-1", "c");

        let events = fanout.drain_session("sess-1");
        assert!(
            events
                .iter()
                .any(|evt| matches!(evt, FanoutEvent::OutputDropped(_))),
            "expected output.dropped telemetry event"
        );
        assert!(
            events
                .iter()
                .any(|evt| matches!(evt, FanoutEvent::Output(_))),
            "expected output event to remain in queue"
        );
    }

    #[test]
    fn summary_exposes_inactive_session_counters() {
        let mut fanout = LocalEventFanout::new(4);
        fanout.publish_output("sess-2", "x");
        fanout.publish_output("sess-2", "y");

        let summary = fanout.summary("sess-2");
        assert_eq!(summary.queued_events, 2);
    }
}
