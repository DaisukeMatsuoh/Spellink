use std::process::ExitCode;

const USAGE: &str = "usage:
  spellink-bridge serve                 run the bridge
  spellink-bridge pair [out.json]       print the connection file for the plugin (one shared token)
  spellink-bridge check [url...]        reach this bridge the way the plugin does (encrypted handshake)
  spellink-bridge open                  open the Spellink Bridge App (status and devices) in your browser
  spellink-bridge device <command>      add, pair, list and revoke devices one by one (run with no command for help)
  spellink-bridge relay [--listen ADDR] [--trust-proxy]   run a relay that lets plugins reach bridges behind NAT
  spellink-bridge version
  (more commands are being ported from the Go bridge: docs/notes/progress/rust-port.md)";

#[tokio::main]
async fn main() -> ExitCode {
    let (cmd, flags, rest) = spellink_bridge::measure::parse_args();
    let result = match cmd.as_str() {
        "serve" => spellink_bridge::app::serve().await,
        "pair" => spellink_bridge::cli::pair(rest.first().map(String::as_str)),
        "check" => spellink_bridge::cli::check(&rest).await,
        "open" => spellink_bridge::cli::open_app().await,
        "device" => spellink_bridge::cli::device(&std::env::args().skip(2).collect::<Vec<_>>()).await,
        "relay" => spellink_bridge::relay::run_cli(&flags).await,
        "version" => {
            println!("{}", spellink_bridge::app::VERSION);
            Ok(())
        }
        "compose" | "ask" | "bench" | "idle" => spellink_bridge::measure::run(&cmd, &flags, &rest),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
