use std::process::ExitCode;

const USAGE: &str = "usage:
  spellink-bridge relay [--listen ADDR] [--trust-proxy]   run a relay that lets plugins reach bridges behind NAT
  spellink-bridge compose|ask|bench|idle ...               measurement commands (see src/measure.rs)";

#[tokio::main]
async fn main() -> ExitCode {
    let (cmd, flags, rest) = spellink_bridge::measure::parse_args();
    let result = match cmd.as_str() {
        "relay" => spellink_bridge::relay::run_cli(&flags).await,
        "compose" | "ask" | "bench" | "idle" => spellink_bridge::measure::run(&cmd, &flags, &rest),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
