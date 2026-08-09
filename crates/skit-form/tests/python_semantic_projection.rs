use skit_form::{CliFormProjection, cli_form_projection};
use skit_language::DegradationReason;

#[test]
fn python_projection_keeps_absent_static_zero_and_dynamic_distinct() {
    assert_eq!(
        cli_form_projection("python", "print('plain')\n"),
        CliFormProjection::Absent
    );
    assert!(matches!(
        cli_form_projection("python", "p.add_argument('--help', action='help')\n"),
        CliFormProjection::Static { framework, fields }
            if framework == "argparse" && fields.is_empty()
    ));
    assert!(matches!(
        cli_form_projection(
            "python",
            "p.add_argument('--x')\np.add_subparsers()\n"
        ),
        CliFormProjection::Dynamic { framework, reason }
            if framework == "argparse" && reason == DegradationReason::Subcommands
    ));
}
