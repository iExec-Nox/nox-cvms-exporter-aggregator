use std::collections::HashMap;

use crate::handlers::CvmSummary;

/// Merges CVM groups collected from several exporters into a single list keyed
/// by `app_id`, concatenating the instances of every group sharing the same
/// `app_id`. For a given `app_id`, the `name` of the first group encountered is
/// kept. Output ordering is unspecified.
pub fn merge_cvms(summaries: impl IntoIterator<Item = CvmSummary>) -> Vec<CvmSummary> {
    let mut groups: HashMap<String, CvmSummary> = HashMap::new();

    for summary in summaries {
        groups
            .entry(summary.app_id.clone())
            .or_insert_with(|| CvmSummary {
                app_id: summary.app_id,
                name: summary.name,
                instances: Vec::new(),
            })
            .instances
            .extend(summary.instances);
    }

    groups.into_values().collect()
}
