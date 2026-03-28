use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnvelope {
    pub event_id: String,
    pub session_id: String,
    pub session_seq: u64,
    pub correlation_id: String,
    pub emitted_at: String,
    pub schema_version: u16,
    pub event_type: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestResult {
    Accepted,
    Duplicate,
    OutOfOrder { expected: u64, actual: u64 },
}

#[derive(Debug, Default)]
pub struct EventIngestor {
    seen_event_ids: HashSet<String>,
    last_seq_by_session: HashMap<String, u64>,
    accepted_events: Vec<EventEnvelope>,
    reconciliation_events: Vec<EventEnvelope>,
}

impl EventIngestor {
    pub fn ingest(&mut self, event: EventEnvelope) -> IngestResult {
        if !self.seen_event_ids.insert(event.event_id.clone()) {
            return IngestResult::Duplicate;
        }

        let expected = self
            .last_seq_by_session
            .get(&event.session_id)
            .copied()
            .map_or(1, |last| last + 1);

        if event.session_seq != expected {
            self.reconciliation_events.push(event.clone());
            return IngestResult::OutOfOrder {
                expected,
                actual: event.session_seq,
            };
        }

        self.last_seq_by_session
            .insert(event.session_id.clone(), event.session_seq);
        self.accepted_events.push(event);
        IngestResult::Accepted
    }

    pub fn accepted_events(&self) -> &[EventEnvelope] {
        &self.accepted_events
    }

    pub fn reconciliation_events(&self) -> &[EventEnvelope] {
        &self.reconciliation_events
    }
}

#[cfg(test)]
mod tests {
    use super::{EventEnvelope, EventIngestor, IngestResult};

    fn event(event_id: &str, session_id: &str, session_seq: u64) -> EventEnvelope {
        EventEnvelope {
            event_id: event_id.to_string(),
            session_id: session_id.to_string(),
            session_seq,
            correlation_id: "corr-1".to_string(),
            emitted_at: "2026-01-01T00:00:00Z".to_string(),
            schema_version: 1,
            event_type: "session.started".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    #[test]
    fn duplicate_event_is_ignored() {
        let mut ingestor = EventIngestor::default();

        let first = event("evt-1", "sess-1", 1);
        let dup = event("evt-1", "sess-1", 1);

        assert_eq!(ingestor.ingest(first), IngestResult::Accepted);
        assert_eq!(ingestor.ingest(dup), IngestResult::Duplicate);
        assert_eq!(ingestor.accepted_events().len(), 1);
    }

    #[test]
    fn out_of_order_event_is_flagged() {
        let mut ingestor = EventIngestor::default();

        assert_eq!(
            ingestor.ingest(event("evt-1", "sess-1", 1)),
            IngestResult::Accepted
        );
        assert_eq!(
            ingestor.ingest(event("evt-2", "sess-1", 3)),
            IngestResult::OutOfOrder {
                expected: 2,
                actual: 3
            }
        );
        assert_eq!(ingestor.accepted_events().len(), 1);
        assert_eq!(ingestor.reconciliation_events().len(), 1);
    }
}
