use std::collections::HashMap;

use crate::types::Summary;

/// Merges per-app CVM groups into a single list keyed by `app_id`, concatenating
/// the instances of every group sharing the same `app_id`. For a given `app_id`,
/// the `name` of the first group encountered is kept. Output ordering is
/// unspecified.
///
/// Generic over the instance type, so it regroups both the plain listing and the
/// enriched response. Each input group typically holds a single instance (one per
/// `(exporter, instance)` pair), and this fold is also where the cross-exporter
/// merge happens.
pub fn merge_cvms<I>(summaries: impl IntoIterator<Item = Summary<I>>) -> Vec<Summary<I>> {
    let mut groups: HashMap<String, Summary<I>> = HashMap::new();

    for summary in summaries {
        groups
            .entry(summary.app_id.clone())
            .or_insert_with(|| Summary {
                app_id: summary.app_id,
                name: summary.name,
                instances: Vec::new(),
            })
            .instances
            .extend(summary.instances);
    }

    groups.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EnrichedCvmInstance, EnrichedCvmSummary, QuoteResponse};

    /// Builds an enriched instance; only the ids matter for these grouping tests,
    /// so the quote/compose fields are left empty.
    fn instance(id: &str, machine: &str) -> EnrichedCvmInstance {
        EnrichedCvmInstance {
            instance_id: id.to_owned(),
            machine_id: machine.to_owned(),
            quote: QuoteResponse {
                quote: String::new(),
                event_log: String::new(),
            },
            app_compose: String::new(),
        }
    }

    fn summary(
        app_id: &str,
        name: &str,
        instances: Vec<EnrichedCvmInstance>,
    ) -> EnrichedCvmSummary {
        EnrichedCvmSummary {
            app_id: app_id.to_owned(),
            name: name.to_owned(),
            instances,
        }
    }

    /// Looks up the merged group for `app_id`, failing the test if it is missing.
    fn group<'a>(merged: &'a [EnrichedCvmSummary], app_id: &str) -> &'a EnrichedCvmSummary {
        merged
            .iter()
            .find(|s| s.app_id == app_id)
            .unwrap_or_else(|| panic!("expected a group for app_id {app_id}"))
    }

    /// Projects a group's instances to sorted `(instance_id, machine_id)` pairs,
    /// so tests can assert membership without needing `PartialEq` on the struct.
    fn instance_keys(s: &EnrichedCvmSummary) -> Vec<(&str, &str)> {
        let mut keys: Vec<(&str, &str)> = s
            .instances
            .iter()
            .map(|i| (i.instance_id.as_str(), i.machine_id.as_str()))
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn distinct_app_ids_stay_separate() {
        // Two exporters, each reporting a different app.
        let exporter_a = vec![summary("app-1", "alpha", vec![instance("i1", "machine-a")])];
        let exporter_b = vec![summary("app-2", "beta", vec![instance("i2", "machine-b")])];

        let merged = merge_cvms(exporter_a.into_iter().chain(exporter_b));

        assert_eq!(merged.len(), 2);
        assert_eq!(
            instance_keys(group(&merged, "app-1")),
            vec![("i1", "machine-a")]
        );
        assert_eq!(
            instance_keys(group(&merged, "app-2")),
            vec![("i2", "machine-b")]
        );
    }

    #[test]
    fn same_app_id_across_exporters_concatenates_instances() {
        // The same app runs on three machines; each exporter reports its own instance.
        let exporter_a = vec![summary("app-1", "alpha", vec![instance("i1", "machine-a")])];
        let exporter_b = vec![summary("app-1", "alpha", vec![instance("i2", "machine-b")])];
        let exporter_c = vec![summary("app-1", "alpha", vec![instance("i3", "machine-c")])];

        let merged = merge_cvms(exporter_a.into_iter().chain(exporter_b).chain(exporter_c));

        assert_eq!(merged.len(), 1);
        let app = group(&merged, "app-1");
        assert_eq!(app.name, "alpha");
        assert_eq!(
            instance_keys(app),
            vec![
                ("i1", "machine-a"),
                ("i2", "machine-b"),
                ("i3", "machine-c")
            ]
        );
    }
}
