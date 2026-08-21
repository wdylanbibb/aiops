use std::time::Duration;

use clap::{Args, Parser, Subcommand};

/// Collect and diagnose Kubernetes incidents.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "aiops", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum Command {
    /// Collect observations from a Kubernetes resource.
    Collect(CollectArgs),
}

#[derive(Debug, Args, PartialEq, Eq)]
pub(crate) struct CollectArgs {
    #[command(subcommand)]
    pub resource: ResourceCommand,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum ResourceCommand {
    /// Collect health, events, and logs for a pod.
    Pod(PodArgs),
}

#[derive(Debug, Args, PartialEq, Eq)]
pub(crate) struct PodArgs {
    /// Name of the pod to collect.
    pub name: String,

    /// Kubernetes namespace containing the pod.
    #[arg(short, long, default_value = "default")]
    pub namespace: String,

    /// How far back to collect observations (for example: 30s, 15m, 2h, 1d).
    #[arg(long, visible_alias = "since", default_value = "15m", value_parser = parse_duration)]
    pub lookback: Duration,

    /// Maximum number of log lines to collect per container.
    #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(1..))]
    pub tail_lines: u64,

    /// Do not collect container logs.
    #[arg(long)]
    pub no_logs: bool,

    /// Do not collect logs from a previously terminated container.
    #[arg(long)]
    pub no_previous_logs: bool,

    /// Do not collect Kubernetes events.
    #[arg(long)]
    pub no_events: bool,
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let (unit, amount) = value
        .as_bytes()
        .split_last()
        .ok_or_else(|| "duration cannot be empty".to_owned())?;

    let amount = std::str::from_utf8(amount)
        .map_err(|_| "duration must contain ASCII digits".to_owned())?
        .parse::<u64>()
        .map_err(|_| "duration must be a positive integer followed by s, m, h, or d".to_owned())?;

    if amount == 0 {
        return Err("duration must be greater than zero".to_owned());
    }

    let seconds_per_unit = match *unit {
        b's' => 1,
        b'm' => 60,
        b'h' => 60 * 60,
        b'd' => 24 * 60 * 60,
        _ => return Err("duration unit must be one of s, m, h, or d".to_owned()),
    };

    amount
        .checked_mul(seconds_per_unit)
        .map(Duration::from_secs)
        .ok_or_else(|| "duration is too large".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_collect_pod_with_defaults() {
        let cli = Cli::try_parse_from(["aiops", "collect", "pod", "api-0"]).unwrap();

        assert_eq!(
            cli,
            Cli {
                command: Command::Collect(CollectArgs {
                    resource: ResourceCommand::Pod(PodArgs {
                        name: "api-0".to_owned(),
                        namespace: "default".to_owned(),
                        lookback: Duration::from_secs(15 * 60),
                        tail_lines: 500,
                        no_logs: false,
                        no_previous_logs: false,
                        no_events: false,
                    }),
                }),
            }
        );
    }

    #[test]
    fn parses_collect_pod_options() {
        let cli = Cli::try_parse_from([
            "aiops",
            "collect",
            "pod",
            "api-0",
            "--namespace",
            "production",
            "--since",
            "2h",
            "--tail-lines",
            "100",
            "--no-logs",
            "--no-previous-logs",
            "--no-events",
        ])
        .unwrap();

        let Command::Collect(CollectArgs {
            resource: ResourceCommand::Pod(args),
        }) = cli.command;
        assert_eq!(args.namespace, "production");
        assert_eq!(args.lookback, Duration::from_secs(2 * 60 * 60));
        assert_eq!(args.tail_lines, 100);
        assert!(args.no_logs);
        assert!(args.no_previous_logs);
        assert!(args.no_events);
    }

    #[test]
    fn rejects_invalid_duration_and_tail_count() {
        assert!(
            Cli::try_parse_from(["aiops", "collect", "pod", "api-0", "--lookback", "15"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["aiops", "collect", "pod", "api-0", "--tail-lines", "0"]).is_err()
        );
    }
}
