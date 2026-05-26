//! GitHub task, PR, and review-comment lifecycle boundary.

mod review;

pub use review::{
    ReviewCommentRecord, ReviewThreadEvidence, fixed_review_thread_ids_from_verifier_evidence,
    parse_unresolved_review_thread_count, review_comment_records_from_tsv,
    review_comment_to_finding, review_feedback_from_comment_records,
    review_feedback_from_comment_records_with_evidence, review_thread_has_managed_fixed_resolution,
    unresolved_fixed_thread_ids, unresolved_fixed_thread_ids_with_evidence,
    unresolved_outdated_thread_ids,
};
