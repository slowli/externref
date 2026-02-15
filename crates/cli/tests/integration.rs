//! Integration tests for the externref CLI.

#![cfg(unix)] // sh-specific user inputs

use term_transcript::{
    ExitStatus, PtyCommand, ShellOptions,
    svg::{Template, TemplateOptions},
    test::TestConfig,
};
#[cfg(feature = "tracing")]
use test_casing::{decorate, decorators::Retry};

fn template() -> Template {
    Template::new(TemplateOptions {
        window_frame: true,
        ..TemplateOptions::default()
    })
}

fn test_config() -> TestConfig<PtyCommand> {
    let shell_options = ShellOptions::new(PtyCommand::default())
        .with_cargo_path()
        .with_current_dir(env!("CARGO_MANIFEST_DIR"))
        .with_status_check("echo $?", |output| {
            let response = output.to_plaintext().ok()?;
            response.trim().parse().ok().map(ExitStatus)
        });
    TestConfig::new(shell_options).with_template(template())
}

#[cfg(feature = "tracing")]
#[test]
#[decorate(Retry::times(3))] // sometimes, the captured output includes `>` from the input
fn cli_with_tracing() {
    // The WASM module is taken from the end-to-end test. We check it into the version control
    // in order for this test to be autonomous.
    test_config().test(
        "tests/snapshots/with-tracing.svg",
        ["RUST_LOG=externref=info \\\n  \
            externref --drop-fn test::drop -o /dev/null tests/test.wasm"],
    );
}

/// This and the following tests ensure that the error message is human-readable.
#[test]
fn error_processing_module() {
    test_config().test(
        "tests/snapshots/error-processing.svg",
        ["externref --drop-fn test::drop -o /dev/null tests/integration.rs"],
    );
}

#[test]
fn error_specifying_drop_fn() {
    test_config().test(
        "tests/snapshots/error-drop-fn.svg",
        ["externref --drop-fn test_drop -o /dev/null tests/test.wasm"],
    );
}
