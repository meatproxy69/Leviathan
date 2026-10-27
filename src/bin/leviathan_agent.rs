//! Direct newline-JSON agent interface; stdout contains only one JSON result per input line.
fn main() {
    let mut args = std::env::args_os().skip(1);
    let option = args.next();
    if option.as_deref() == Some(std::ffi::OsStr::new("--help")) {
        println!("Usage: leviathan_agent --workspace-root EXISTING_DIRECTORY\nDirect agent JSONL protocol (help, discover, call); isolated headless document. No GUI or audio-device control in this standalone process.");
        return;
    }
    let root = args.next();
    if option.as_deref() != Some(std::ffi::OsStr::new("--workspace-root"))
        || root.is_none()
        || args.next().is_some()
    {
        eprintln!("Usage: leviathan_agent --workspace-root EXISTING_DIRECTORY");
        std::process::exit(2);
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(error) = aqua_pattern_core::mcp::serve_agent(
        std::path::Path::new(&root.unwrap()),
        stdin.lock(),
        stdout.lock(),
    ) {
        eprintln!("Leviathan agent: {error}");
        std::process::exit(1);
    }
}
