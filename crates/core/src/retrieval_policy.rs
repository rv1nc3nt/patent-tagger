//! Pure retrieval-policy eligibility (SPEC 5.5): decides *whether* a
//! validated/auto-completed document should be enqueued for full-text or
//! drawings retrieval, given the setting and (for the "selected tags"
//! policy) which tags the document actually carries. Network access and
//! job queueing are `src-tauri`'s job; "never" and "on demand" both mean
//! "not automatically after tagging" here (on demand is triggered by the
//! document view's own button, not this check).

pub const POLICY_NEVER: &str = "never";
pub const POLICY_ON_DEMAND: &str = "on_demand";
pub const POLICY_AFTER_TAGGING_ALL: &str = "after_tagging_all";
pub const POLICY_AFTER_TAGGING_SELECTED_TAGS: &str = "after_tagging_selected_tags";

pub fn should_retrieve_after_tagging(
    policy: &str,
    policy_tag_ids: &[i64],
    document_tag_ids: &[i64],
) -> bool {
    match policy {
        POLICY_AFTER_TAGGING_ALL => true,
        POLICY_AFTER_TAGGING_SELECTED_TAGS => document_tag_ids
            .iter()
            .any(|id| policy_tag_ids.contains(id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_tagging_all_always_retrieves() {
        assert!(should_retrieve_after_tagging(
            POLICY_AFTER_TAGGING_ALL,
            &[],
            &[]
        ));
        assert!(should_retrieve_after_tagging(
            POLICY_AFTER_TAGGING_ALL,
            &[1, 2],
            &[3]
        ));
    }

    #[test]
    fn never_and_on_demand_never_retrieve_automatically() {
        assert!(!should_retrieve_after_tagging(POLICY_NEVER, &[1], &[1]));
        assert!(!should_retrieve_after_tagging(POLICY_ON_DEMAND, &[1], &[1]));
    }

    #[test]
    fn selected_tags_requires_overlap_with_the_documents_tags() {
        assert!(should_retrieve_after_tagging(
            POLICY_AFTER_TAGGING_SELECTED_TAGS,
            &[1, 2],
            &[2, 3]
        ));
        assert!(!should_retrieve_after_tagging(
            POLICY_AFTER_TAGGING_SELECTED_TAGS,
            &[1, 2],
            &[3, 4]
        ));
        assert!(!should_retrieve_after_tagging(
            POLICY_AFTER_TAGGING_SELECTED_TAGS,
            &[1, 2],
            &[]
        ));
    }
}
