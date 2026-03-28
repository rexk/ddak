use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeMetadata {
    pub runtime_instance_id: String,
    pub adapter_session_ref: Option<String>,
    pub runtime_pid: Option<u32>,
    pub has_resume_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeAttempt {
    pub adapter_session_ref: Option<String>,
    pub runtime_pid: Option<u32>,
    pub has_resume_hint: bool,
    pub operator_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    ResumeAutomatically,
    RequireOperatorConfirmation,
    Reject,
}

pub fn new_runtime_instance_id() -> String {
    Uuid::now_v7().to_string()
}

pub fn confidence_score(meta: &ResumeMetadata, attempt: &ResumeAttempt) -> u8 {
    let mut score = 0_u8;

    if meta.adapter_session_ref.is_some() && meta.adapter_session_ref == attempt.adapter_session_ref
    {
        score = score.saturating_add(60);
    }
    if meta.runtime_pid.is_some() && meta.runtime_pid == attempt.runtime_pid {
        score = score.saturating_add(30);
    }
    if meta.has_resume_hint && attempt.has_resume_hint {
        score = score.saturating_add(10);
    }

    score.min(100)
}

pub fn evaluate_resume(
    meta: &ResumeMetadata,
    attempt: &ResumeAttempt,
    threshold: u8,
) -> ResumeDecision {
    let score = confidence_score(meta, attempt);

    if score >= threshold {
        return ResumeDecision::ResumeAutomatically;
    }

    if attempt.operator_confirmed {
        return ResumeDecision::ResumeAutomatically;
    }

    if score >= 20 {
        ResumeDecision::RequireOperatorConfirmation
    } else {
        ResumeDecision::Reject
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ResumeAttempt, ResumeDecision, ResumeMetadata, confidence_score, evaluate_resume,
        new_runtime_instance_id,
    };

    #[test]
    fn generates_runtime_instance_id() {
        let id = new_runtime_instance_id();
        assert!(!id.is_empty());
        assert!(id.contains('-'));
    }

    #[test]
    fn high_confidence_resumes_automatically() {
        let meta = ResumeMetadata {
            runtime_instance_id: "inst-1".to_string(),
            adapter_session_ref: Some("ref-1".to_string()),
            runtime_pid: Some(42),
            has_resume_hint: true,
        };
        let attempt = ResumeAttempt {
            adapter_session_ref: Some("ref-1".to_string()),
            runtime_pid: Some(42),
            has_resume_hint: true,
            operator_confirmed: false,
        };

        assert!(confidence_score(&meta, &attempt) >= 90);
        assert_eq!(
            evaluate_resume(&meta, &attempt, 80),
            ResumeDecision::ResumeAutomatically
        );
    }

    #[test]
    fn low_confidence_requires_confirmation() {
        let meta = ResumeMetadata {
            runtime_instance_id: "inst-1".to_string(),
            adapter_session_ref: Some("ref-1".to_string()),
            runtime_pid: Some(42),
            has_resume_hint: true,
        };
        let attempt = ResumeAttempt {
            adapter_session_ref: None,
            runtime_pid: Some(42),
            has_resume_hint: false,
            operator_confirmed: false,
        };

        assert_eq!(
            evaluate_resume(&meta, &attempt, 80),
            ResumeDecision::RequireOperatorConfirmation
        );
    }

    #[test]
    fn low_confidence_cannot_proceed_silently() {
        let meta = ResumeMetadata {
            runtime_instance_id: "inst-1".to_string(),
            adapter_session_ref: Some("ref-1".to_string()),
            runtime_pid: Some(42),
            has_resume_hint: false,
        };
        let attempt = ResumeAttempt {
            adapter_session_ref: None,
            runtime_pid: None,
            has_resume_hint: false,
            operator_confirmed: false,
        };

        assert_eq!(
            evaluate_resume(&meta, &attempt, 80),
            ResumeDecision::Reject,
            "low confidence resume should not proceed silently"
        );

        let confirmed = ResumeAttempt {
            operator_confirmed: true,
            ..attempt
        };
        assert_eq!(
            evaluate_resume(&meta, &confirmed, 80),
            ResumeDecision::ResumeAutomatically,
            "explicit operator confirmation should allow guarded resume"
        );
    }
}
