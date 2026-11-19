use clap::Parser;
use v2exe::cli::args::Cli;
use v2exe::cli::run::{RunEnv, run};

fn main() {
    let cli = Cli::parse();
    let env = RunEnv::from_process();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the tokio runtime starts");
    let result = {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        runtime.block_on(run(cli, &mut out, &env))
    };
    if let Err(failure) = result {
        eprintln!("v2exe: {}", failure.message);
        std::process::exit(i32::from(failure.code));
    }
}
