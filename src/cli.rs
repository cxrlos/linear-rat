use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "lrat", version, about)]
pub struct Cli {
    /// Run without a Linear account or API key
    #[arg(long)]
    pub demo: bool,
}
