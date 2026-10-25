use phonic::{outputs::WavOutput, OutputDevice};
fn main() {
    let path = std::env::args().nth(1).expect("output path");
    let mut output = WavOutput::open(path).unwrap();
    eprintln!("calling stop");
    output.stop();
    eprintln!("stop returned");
    output.close();
}
