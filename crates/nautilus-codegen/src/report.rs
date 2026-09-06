//! What a generation run tells the user.
//!
//! Printing lives here so that generating a client does none of it: the
//! backends hand back warnings as text, and the command prints them around the
//! two lines that frame a run — the schema it loaded and the client it wrote.

use std::path::Path;
use std::time::Duration;

use nautilus_schema::ir::SchemaIr;

pub(crate) fn warning(message: &str) {
    eprintln!(
        "{} {}",
        console::style("warning:").yellow().bold(),
        console::style(message).yellow()
    );
}

/// Announce the schema a run loaded, and where its variables came from.
pub(crate) fn loaded_schema(schema_path: &Path, ir: &SchemaIr) {
    if let Some(ds) = &ir.datasource {
        if let Some(var_name) = ds
            .url
            .strip_prefix("env(")
            .and_then(|s| s.strip_suffix(')'))
        {
            // The variable may equally have come from the process environment,
            // and naming `.env` when no such file exists sends the reader
            // looking for one.
            let source = if Path::new(".env").exists() {
                "from .env"
            } else {
                "from the environment"
            };
            println!(
                "{} {} {}",
                console::style("Loaded").dim(),
                console::style(var_name).bold(),
                console::style(source).dim()
            );
        }
    }

    println!(
        "{} {}",
        console::style("Nautilus schema loaded from").dim(),
        console::style(schema_path.display()).italic().dim()
    );
}

/// Announce the finished client and where it landed.
pub(crate) fn generated(language: &str, output: &str, elapsed: Duration) {
    println!(
        "\nGenerated {} {} {} {}\n",
        console::style(format!(
            "Nautilus Client for {} (v{})",
            language,
            env!("CARGO_PKG_VERSION")
        ))
        .bold(),
        console::style("to").dim(),
        console::style(output).italic().dim(),
        console::style(format!("({}ms)", elapsed.as_millis())).italic()
    );
}
