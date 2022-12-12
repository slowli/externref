//! CLI for the `externref` crate.

use anyhow::{anyhow, ensure, Context};
use structopt::StructOpt;

use std::{
    fs,
    io::{self, Read as _, Write as _},
    path::PathBuf,
    str::FromStr,
};

use externref::processor::Processor;

#[derive(Debug)]
struct ModuleAndName {
    module: String,
    name: String,
}

impl FromStr for ModuleAndName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (module, name) = s
            .split_once("::")
            .ok_or_else(|| anyhow!("function must be specified in the `module::name` format"))?;

        ensure!(!module.is_empty(), "module cannot be empty");
        ensure!(module.is_ascii(), "module must contain ASCII chars only");
        ensure!(!name.is_empty(), "name cannot be empty");
        ensure!(name.is_ascii(), "name must contain ASCII chars only");
        Ok(Self {
            module: module.to_owned(),
            name: name.to_owned(),
        })
    }
}

/// CLI for transforming WASM modules with `externref` shims produced with the help
/// of the `externref` crate.
#[derive(Debug, StructOpt)]
struct Args {
    /// Path to the input WASM module.
    /// If set to `-`, the module will be read from the standard input.
    input: PathBuf,
    /// Path to the output WASM module. If not specified, the module will be emitted
    /// to the standard output.
    #[structopt(long, short = "o")]
    output: Option<PathBuf>,
    /// Name of the exported `externref`s table where refs obtained from the host
    /// are placed.
    #[structopt(long = "table", default_value = "externrefs")]
    export_table: String,
    /// Function to notify the host about dropped `externref`s specified
    /// in the `module::name` format.
    ///
    /// This function will be added as an import with a signature `(externref) -> ()`
    /// and will be called immediately before dropping each reference.
    #[structopt(long = "drop-fn")]
    drop_fn: Option<ModuleAndName>,
}

impl Args {
    #[cfg(feature = "tracing")]
    fn configure_tracing() {
        use tracing_subscriber::{filter::EnvFilter, FmtSubscriber};

        FmtSubscriber::builder()
            .without_time()
            .with_env_filter(EnvFilter::from_default_env())
            .init();
    }

    fn run(&self) -> anyhow::Result<()> {
        #[cfg(feature = "tracing")]
        Self::configure_tracing();

        let module = self.read_input_module().with_context(|| {
            format!(
                "failed reading input module from `{}`",
                self.input.to_string_lossy()
            )
        })?;

        let mut processor = Processor::default();
        processor.set_ref_table(self.export_table.as_str());
        if let Some(drop_fn) = &self.drop_fn {
            processor.set_drop_fn(&drop_fn.module, &drop_fn.name);
        }
        let processed = processor
            .process_bytes(&module)
            .context("failed processing module")?;

        self.write_output_module(&processed).with_context(|| {
            if let Some(path) = &self.output {
                format!("failed writing module to file `{}`", path.to_string_lossy())
            } else {
                "failed writing module to standard output".to_owned()
            }
        })
    }

    fn read_input_module(&self) -> anyhow::Result<Vec<u8>> {
        let bytes = if self.input.as_os_str() == "-" {
            let mut buffer = Vec::with_capacity(1_024);
            io::stdin().read_to_end(&mut buffer)?;
            buffer
        } else {
            fs::read(&self.input)?
        };
        Ok(bytes)
    }

    fn write_output_module(&self, bytes: &[u8]) -> anyhow::Result<()> {
        if let Some(path) = &self.output {
            fs::write(path, bytes)?;
        } else {
            io::stdout().lock().write_all(bytes)?;
        }
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    Args::from_args().run()
}
