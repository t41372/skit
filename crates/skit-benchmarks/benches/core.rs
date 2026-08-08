use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use ratatui_core::{backend::TestBackend, terminal::Terminal};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_language::detect_candidates;
use skit_tui::render;
use skit_ui::{Action, LibraryState};

fn library(size: usize) -> LibraryState {
    let entries = (0..size)
        .map(|index| EntrySummary {
            slug: Slug::parse(format!("entry-{index}")).expect("benchmark slug"),
            name: format!("Entry {index}"),
            kind: EntryKind::parse(if index % 2 == 0 { "python" } else { "shell" })
                .expect("benchmark kind"),
            mode: StorageMode::Copy,
            description: format!("Benchmark entry {index}"),
            target: None,
        })
        .collect();
    LibraryState::from_scan(LibraryScan {
        entries,
        diagnostics: Vec::new(),
    })
}

fn benchmarks(criterion: &mut Criterion) {
    let python = "CITY = 'Taipei'\nname = input('Name: ')\n".repeat(200);
    let shell = "CITY=Taipei\nread -p 'Name: ' NAME\n".repeat(200);
    let javascript = "const CITY = 'Taipei';\n".repeat(200);

    criterion.bench_function("analyze_python_400_lines", |bencher| {
        bencher.iter(|| detect_candidates("python", black_box(&python)))
    });
    criterion.bench_function("analyze_shell_400_lines", |bencher| {
        bencher.iter(|| detect_candidates("shell", black_box(&shell)))
    });
    criterion.bench_function("analyze_javascript_200_lines", |bencher| {
        bencher.iter(|| detect_candidates("js", black_box(&javascript)))
    });

    let mut state = library(1_000);
    criterion.bench_function("filter_1000_entries", |bencher| {
        bencher.iter(|| {
            state.update(Action::BeginSearch);
            state.update(Action::ClearSearch);
            state.update(Action::Input(black_box('9')));
            black_box(state.visible_entries().len())
        })
    });

    let state = library(1_000);
    criterion.bench_function("render_1000_entries", |bencher| {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("benchmark terminal");
        bencher.iter(|| {
            terminal
                .draw(|frame| {
                    black_box(render(frame, black_box(&state)));
                })
                .expect("benchmark draw");
        });
    });
}

criterion_group!(core, benchmarks);
criterion_main!(core);
