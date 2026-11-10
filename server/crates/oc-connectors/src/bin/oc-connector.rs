//! `oc-connector`: checks connector packages with the server's own code (scripts/connector.mjs calls it).
//!
//!   oc-connector validate <dir>...   the manifest and its files
//!   oc-connector test <dir>...       validate, then replay the recorded answers (OC_CONNECTOR_UPDATE=1 rewrites golden files)
//!   oc-connector schema              the manifest's JSON Schema

use std::path::Path;
use std::process::ExitCode;

use oc_connectors::package::{Origin, Package};

fn load(dir: &str) -> Result<Package, ()> {
    Package::from_dir(Path::new(dir), Origin::ThirdParty { dir: dir.to_string() }).map_err(|problems| {
        eprintln!("{dir}: invalid");
        for p in problems {
            eprintln!("  {p}");
        }
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprintln!("usage: oc-connector validate|test <dir>... | schema");
        return ExitCode::from(2);
    };
    let dirs = &args[1..];
    match cmd.as_str() {
        "schema" => {
            print!("{}", oc_connectors::schema::json_schema_text());
            ExitCode::SUCCESS
        }
        "validate" | "test" if !dirs.is_empty() => {
            let update = std::env::var("OC_CONNECTOR_UPDATE").is_ok_and(|v| v == "1");
            let mut failed = false;
            for dir in dirs {
                let Ok(pkg) = load(dir) else {
                    failed = true;
                    continue;
                };
                if cmd == "validate" {
                    println!("{dir}: valid ({} {})", pkg.manifest.id, pkg.manifest.version);
                    continue;
                }
                let report = oc_connectors::replay::replay(&pkg, update).await;
                for line in &report.summary {
                    println!("{dir}: {line}");
                }
                for (path, text) in &report.updates {
                    let target = Path::new(dir).join(path);
                    if let Err(e) = std::fs::write(&target, text) {
                        eprintln!("{}: {e}", target.display());
                        failed = true;
                    } else {
                        println!("{dir}: wrote {path}");
                    }
                }
                if report.problems.is_empty() {
                    println!("{dir}: ok");
                } else {
                    failed = true;
                    for p in &report.problems {
                        eprintln!("{dir}: {p}");
                    }
                }
            }
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        _ => {
            eprintln!("usage: oc-connector validate|test <dir>... | schema");
            ExitCode::from(2)
        }
    }
}
