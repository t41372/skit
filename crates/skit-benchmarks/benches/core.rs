use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use ratatui_core::{backend::TestBackend, terminal::Terminal};
use skit_application::{EntryRepository as _, LibraryScan, LibraryService};
use skit_benchmarks::{
    dataset::{dataset_dirs, generate, generate_command_only},
    sources::{LANGUAGES, generate as generate_source},
};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_language::detect_candidates;
use skit_store::FileStore;
use skit_tui::render;
use skit_ui::{Action, LibraryState};
use tempfile::TempDir;

const STORE_N: usize = 200;

struct StoreFixture {
    _root: TempDir,
    store: FileStore,
    slugs: Vec<Slug>,
}

fn store_fixture(command_only: bool) -> StoreFixture {
    let root = TempDir::new().expect("benchmark directory");
    let dataset_root = root.path().join("dataset");
    let manifest = if command_only {
        generate_command_only(dataset_root, STORE_N).expect("command benchmark library")
    } else {
        generate(
            &dataset_root,
            STORE_N as isize,
            skit_benchmarks::dataset::DEFAULT_SEED,
            skit_benchmarks::dataset::DEFAULT_STATE_FRACTION,
        )
        .expect("mixed benchmark library")
    };
    let store = FileStore::new(dataset_dirs(&manifest.root).expect("dataset paths").data);
    StoreFixture {
        _root: root,
        store,
        slugs: manifest.slugs,
    }
}

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

    for language in LANGUAGES {
        for lines in [20, 200, 2_000] {
            let source = generate_source(language, lines).expect("benchmark source");
            criterion.bench_function(&format!("analyze/{language}/l{lines}"), |bencher| {
                bencher.iter(|| detect_candidates(language, black_box(&source)));
            });
        }
    }

    for (shape, fixture) in [
        ("commands", store_fixture(true)),
        ("mixed", store_fixture(false)),
    ] {
        let service = LibraryService::new(fixture.store.clone());
        criterion.bench_function(&format!("store/list_entries/{shape}"), |bencher| {
            bencher.iter(|| {
                let scan = fixture.store.scan().expect("scan benchmark library");
                let entries = scan
                    .entries
                    .into_iter()
                    .map(|summary| {
                        fixture
                            .store
                            .resolve(summary.slug.as_str())
                            .expect("resolve benchmark entry")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(black_box(entries).len(), STORE_N);
            });
        });
        criterion.bench_function(&format!("store/list_summaries/{shape}"), |bencher| {
            bencher.iter(|| {
                let scan = service.list().expect("list benchmark library");
                assert_eq!(black_box(scan.entries).len(), STORE_N);
            });
        });
        let target = fixture.slugs.last().expect("non-empty fixture");
        criterion.bench_function(&format!("store/resolve/{shape}"), |bencher| {
            bencher.iter(|| {
                let entry = fixture
                    .store
                    .resolve(black_box(target.as_str()))
                    .expect("resolve benchmark entry");
                assert_eq!(&entry.slug, target);
            });
        });
    }

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
