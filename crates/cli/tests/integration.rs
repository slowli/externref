//! Integration tests for the externref CLI.

#![cfg(unix)] // sh-specific input

use term_transcript::{
    svg::{Template, TemplateOptions},
    test::TestConfig,
    PtyCommand, ShellOptions,
};

fn template() -> Template {
    Template::new(TemplateOptions {
        window_frame: true,
        ..TemplateOptions::default()
    })
}

fn test_config() -> TestConfig<PtyCommand> {
    let shell_options = ShellOptions::new(PtyCommand::default())
        .with_cargo_path()
        .with_current_dir(env!("CARGO_MANIFEST_DIR"));
    TestConfig::new(shell_options)
}

#[test]
fn cli_basics() {
    // The WASM module is taken from the end-to-end test. We check it into the version control
    // in order for this test to be autonomous.
    test_config().with_template(template()).test(
        "tests/snapshots/with-tracing.svg",
        ["RUST_LOG=externref=info \\\n  \
            externref --drop-fn test::drop -o /dev/null tests/test.wasm"],
    );
}
