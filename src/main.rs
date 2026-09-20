mod cli;
mod frame_parser;
mod report_renderer;
mod rule_engine;

fn main() -> anyhow::Result<()> {
    cli::run()
}
