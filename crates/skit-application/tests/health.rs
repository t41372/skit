use std::{cell::Cell, fmt};

use skit_application::health::{
    HealthInspection, HealthIssue, HealthIssueKind, HealthRebuild, HealthRebuildOutcome,
    HealthService, HealthSnapshot, MirrorHealth, UvHealth,
};

#[derive(Debug, Eq, PartialEq)]
struct InspectError;

impl fmt::Display for InspectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("inspection failed")
    }
}

#[derive(Debug)]
struct Inspector {
    rebuilds: Cell<usize>,
}

impl HealthInspection for Inspector {
    type Error = InspectError;

    fn inspect(&self) -> Result<HealthSnapshot, Self::Error> {
        Ok(snapshot(2))
    }

    fn rebuild(&self) -> Result<HealthRebuild, Self::Error> {
        self.rebuilds.set(self.rebuilds.get() + 1);
        Ok(HealthRebuild {
            snapshot: snapshot(3),
            outcome: HealthRebuildOutcome {
                entry_count: 3,
                problems: vec!["orphan: meta.toml is missing; skipped".to_owned()],
            },
        })
    }
}

fn snapshot(entry_count: usize) -> HealthSnapshot {
    HealthSnapshot {
        uv: UvHealth::Found("/usr/bin/uv".to_owned()),
        entry_count,
        issues: vec![HealthIssue {
            slug: "broken".to_owned(),
            name: "Broken".to_owned(),
            kind: HealthIssueKind::MissingTarget,
        }],
        invalid_runner_rows: vec!["row 2".to_owned()],
        mirror: MirrorHealth::Off,
        library_path: "/data/scripts".to_owned(),
        library_size: "2 KiB".to_owned(),
        diagnostics: Vec::new(),
    }
}

#[test]
fn one_inspection_port_supplies_initial_and_post_rebuild_snapshots() {
    let service = HealthService::new(Inspector {
        rebuilds: Cell::new(0),
    });

    assert_eq!(service.inspect().unwrap().entry_count, 2);
    let rebuilt = service.rebuild().unwrap();
    assert_eq!(rebuilt.snapshot.entry_count, 3);
    assert_eq!(rebuilt.outcome.entry_count, 3);
    assert_eq!(rebuilt.outcome.problems.len(), 1);
    assert_eq!(service.inspector().rebuilds.get(), 1);
}
