//! Device-free diagnostics. This executable never enables or opens an audio backend.
#[path = "../tests/support/transport_diagnostics.rs"]
mod diagnostics;

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 {
        eprintln!("usage: transport_stress ABSOLUTE_NEW_DIRECTORY (run under an external timeout)");
        std::process::exit(2);
    }
    match diagnostics::run(std::path::Path::new(&args[1])) {
        Ok(report) => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
        Err(error) => {
            eprintln!("transport diagnostics FAILED / incomplete: {error}; created evidence stays on disk");
            std::process::exit(1);
        }
    }
}
