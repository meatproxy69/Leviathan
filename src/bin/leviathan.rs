fn main() -> eframe::Result {
    let argument = std::env::args().nth(1);
    if argument
        .as_deref()
        .is_some_and(|a| a == "--version" || a == "-V")
    {
        println!("LEVIATHAN {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if argument
        .as_deref()
        .is_some_and(|a| a == "--help" || a == "-h")
    {
        println!("LEVIATHAN {}\nUsage: leviathan [--project PATH]\n       leviathan --agent-stdio --workspace-root DIRECTORY [--project PATH]\nUse Samples to load a WAV or FLAC file.\n--agent-stdio   Expose this running window and its document through local JSONL stdin/stdout\n--workspace-root  Root for structured project operations and artifact outputs\n--help    Show this help\n--version Show version", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|arg| arg == "--agent-stdio") {
        let valid = (|| -> Result<(), String> {
            let mut index = 0;
            let mut root = None;
            let mut agent = false;
            let mut project = false;
            while index < args.len() {
                if args[index] == "--agent-stdio" && !agent {
                    agent = true;
                    index += 1;
                } else if args[index] == "--workspace-root" && root.is_none() {
                    index += 1;
                    root = Some(
                        args.get(index)
                            .ok_or("--workspace-root requires a directory")?,
                    );
                    index += 1;
                } else if args[index] == "--project" && !project {
                    index += 1;
                    args.get(index).ok_or("--project requires a path")?;
                    project = true;
                    index += 1;
                } else {
                    return Err("Agent mode accepts --agent-stdio, --workspace-root DIRECTORY and optional --project PATH".into());
                }
            }
            if !std::path::Path::new(
                root.ok_or("--agent-stdio requires --workspace-root DIRECTORY")?,
            )
            .is_dir()
            {
                return Err("Workspace root must be an existing directory".into());
            }
            Ok(())
        })();
        if let Err(error) = valid {
            eprintln!("Leviathan agent: {error}");
            std::process::exit(2);
        }
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([900.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Leviathan",
        options,
        Box::new(|cc| Ok(Box::new(aqua_pattern_core::desktop::Leviathan::new(cc)))),
    )
}
