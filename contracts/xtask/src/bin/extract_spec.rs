//! CLI tool for extracting Soroban contract specifications from compiled WASMs (#937).
//!
//! Usage:
//!   cargo run -p kora-xtask --bin extract-spec -- <wasm_files...> [--out <file>] [--format json|interface]
//!   cargo run -p kora-xtask --bin extract-spec -- --dir target/wasm32-unknown-unknown/release/ [--out <dir|file>]

use kora_xtask::spec::{
    extract_spec_from_file, format_spec_interface_text, spec_to_deterministic_json, ContractSpec,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn print_help() {
    println!(
        r#"Soroban Contract Spec Extractor (Issue #937)

Extracts ABI and contract specifications (contractspecv0) from compiled Soroban WASM files.

USAGE:
    extract-spec [OPTIONS] [WASM_FILES...]

OPTIONS:
    --dir <DIRECTORY>       Process all .wasm files in the specified directory
    --out <PATH>            Output file or directory (default: stdout)
    --format <json|text>    Output format: 'json' (default) or 'text' / 'interface'
    --help, -h              Print this help information

EXAMPLES:
    # Extract JSON spec to stdout:
    cargo run -p kora-xtask --bin extract-spec -- target/wasm32-unknown-unknown/release/kora_invoice_nft.wasm

    # Extract all contract specs in release dir to specs/ directory:
    cargo run -p kora-xtask --bin extract-spec -- --dir target/wasm32-unknown-unknown/release --out specs/ --format text
"#
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        print_help();
        return ExitCode::SUCCESS;
    }

    let mut wasm_paths: Vec<PathBuf> = Vec::new();
    let mut out_path: Option<PathBuf> = None;
    let mut format = "json".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --dir requires a directory path");
                    return ExitCode::FAILURE;
                }
                let dir = Path::new(&args[i]);
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("wasm") {
                            wasm_paths.push(path);
                        }
                    }
                } else {
                    eprintln!("Error reading directory: {dir:?}");
                    return ExitCode::FAILURE;
                }
            }
            "--out" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --out requires a file or directory path");
                    return ExitCode::FAILURE;
                }
                out_path = Some(PathBuf::from(&args[i]));
            }
            "--format" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --format requires 'json' or 'text'");
                    return ExitCode::FAILURE;
                }
                format = args[i].to_lowercase();
            }
            arg if !arg.starts_with('-') => {
                wasm_paths.push(PathBuf::from(arg));
            }
            other => {
                eprintln!("Unknown argument: {other}");
                print_help();
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    if wasm_paths.is_empty() {
        eprintln!("No WASM files provided or found.");
        return ExitCode::FAILURE;
    }

    // Sort paths for determinism
    wasm_paths.sort();

    let mut specs: Vec<ContractSpec> = Vec::new();
    for p in &wasm_paths {
        match extract_spec_from_file(p) {
            Ok(spec) => {
                eprintln!("✓ Extracted spec for '{}' from {:?}", spec.name, p.file_name().unwrap_or_default());
                specs.push(spec);
            }
            Err(e) => {
                eprintln!("Warning: could not extract spec from {:?}: {e}", p);
            }
        }
    }

    if specs.is_empty() {
        eprintln!("No valid contract specifications could be extracted.");
        return ExitCode::FAILURE;
    }

    // Handle output
    if let Some(ref dest) = out_path {
        if dest.is_dir() || (specs.len() > 1 && dest.extension().is_none()) {
            let _ = fs::create_dir_all(dest);
            for spec in &specs {
                let ext = if format == "text" || format == "interface" { "txt" } else { "json" };
                let file_path = dest.join(format!("{}.{ext}", spec.name));
                let content = if format == "text" || format == "interface" {
                    format_spec_interface_text(spec)
                } else {
                    spec_to_deterministic_json(spec).expect("JSON serialization failed")
                };
                if let Err(e) = fs::write(&file_path, content) {
                    eprintln!("Failed to write {file_path:?}: {e}");
                    return ExitCode::FAILURE;
                }
                eprintln!("Saved {}", file_path.display());
            }
            return ExitCode::SUCCESS;
        }

        // Single file output
        let content = if format == "text" || format == "interface" {
            specs.iter().map(format_spec_interface_text).collect::<Vec<_>>().join("\n---\n\n")
        } else if specs.len() == 1 {
            spec_to_deterministic_json(&specs[0]).expect("JSON serialization failed")
        } else {
            serde_json::to_string_pretty(&specs).expect("JSON serialization failed")
        };

        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Err(e) = fs::write(dest, content) {
            eprintln!("Failed to write to {dest:?}: {e}");
            return ExitCode::FAILURE;
        }
        eprintln!("Saved {}", dest.display());
    } else {
        // Stdout
        for spec in &specs {
            if format == "text" || format == "interface" {
                println!("{}", format_spec_interface_text(spec));
            } else if specs.len() == 1 {
                println!("{}", spec_to_deterministic_json(spec).expect("JSON failed"));
            } else {
                println!("{}", serde_json::to_string_pretty(&specs).expect("JSON failed"));
                break;
            }
        }
    }

    ExitCode::SUCCESS
}
