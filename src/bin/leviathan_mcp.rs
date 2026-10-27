//! Explicit-root local MCP subprocess; stdout is reserved for JSON-RPC.
fn main() {
    let mut args = std::env::args_os().skip(1);
    let option = args.next();
    if option.as_deref() == Some(std::ffi::OsStr::new("--help")) {
        eprintln!("Usage: leviathan_mcp --workspace-root EXISTING_DIRECTORY\nLocal stdio MCP; isolated headless document; no GUI/device control.");
        return;
    }
    let root = args.next();
    if option.as_deref() != Some(std::ffi::OsStr::new("--workspace-root"))
        || root.is_none()
        || args.next().is_some()
    {
        eprintln!("Usage: leviathan_mcp --workspace-root EXISTING_DIRECTORY");
        std::process::exit(2);
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(error) = aqua_pattern_core::mcp::serve(
        std::path::Path::new(&root.unwrap()),
        stdin.lock(),
        stdout.lock(),
    ) {
        eprintln!("Leviathan MCP: {error}");
        std::process::exit(1);
    }
}
