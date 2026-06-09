use std::collections::HashMap;

use crate::types::CvmSummary;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CvmInstance;

    /// Builds an instance whose fields encode the exporter it came from, so tests
    /// can assert that instances were carried over from the right machine.
    fn instance(id: &str, machine: &str) -> CvmInstance {
        CvmInstance {
            instance_id: id.to_owned(),
            url: format!("https://{id}.example"),
            machine_id: machine.to_owned(),
        }
    }

    fn summary(app_id: &str, name: &str, instances: Vec<CvmInstance>) -> CvmSummary {
        CvmSummary {
            app_id: app_id.to_owned(),
            name: name.to_owned(),
            instances,
        }
    }

    /// Looks up the merged group for `app_id`, failing the test if it is missing.
    fn group<'a>(merged: &'a [CvmSummary], app_id: &str) -> &'a CvmSummary {
        merged
            .iter()
            .find(|s| s.app_id == app_id)
            .unwrap_or_else(|| panic!("expected a group for app_id {app_id}"))
    }

    #[test]
    fn distinct_app_ids_stay_separate() {
        // Two exporters, each reporting a different app.
        let exporter_a = vec![summary("app-1", "alpha", vec![instance("i1", "machine-a")])];
        let exporter_b = vec![summary("app-2", "beta", vec![instance("i2", "machine-b")])];

        let merged = merge_cvms(exporter_a.into_iter().chain(exporter_b));

        assert_eq!(merged.len(), 2);
        assert_eq!(
            group(&merged, "app-1").instances,
            vec![instance("i1", "machine-a")]
        );
        assert_eq!(
            group(&merged, "app-2").instances,
            vec![instance("i2", "machine-b")]
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
        assert_eq!(app.instances.len(), 3);

        let mut machines: Vec<&str> = app
            .instances
            .iter()
            .map(|i| i.machine_id.as_str())
            .collect();
        machines.sort_unstable();
        assert_eq!(machines, vec!["machine-a", "machine-b", "machine-c"]);
    }
}
