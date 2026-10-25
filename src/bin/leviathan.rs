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
        println!("LEVIATHAN {}\nUsage: leviathan [--project PATH]\nNo samples are bundled. Assign your own files in Audio / Export.\n--help    Show this help\n--version Show version",env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([900.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        concat!("LEVIATHAN ", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(|cc| Ok(Box::new(aqua_pattern_core::desktop::Leviathan::new(cc)))),
    )
}
