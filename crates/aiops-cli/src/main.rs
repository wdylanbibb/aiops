mod args;

use std::io::Write;

use aiops_k8s::{
    client::KubernetesClient,
    collector::{CollectionOptions, KubernetesCollector},
};
use anyhow::Context;
use clap::Parser;

use crate::args::{CollectArgs, Command, PodArgs, ResourceCommand};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = args::Cli::parse();

    match cli.command {
        Command::Collect(CollectArgs {
            resource: ResourceCommand::Pod(args),
        }) => collect_pod(args).await?,
    }

    Ok(())
}

async fn collect_pod(args: PodArgs) -> anyhow::Result<()> {
    let options = collection_options(&args)?;
    let client = KubernetesClient::infer()
        .await
        .context("failed to connect to Kubernetes")?;
    let collector = KubernetesCollector::new(client, options);
    let bundle = collector
        .collect_pod(&args.namespace, &args.name)
        .await
        .with_context(|| format!("failed to collect pod {}/{}", args.namespace, args.name))?;

    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &bundle)
        .context("failed to write observation bundle")?;
    writeln!(output).context("failed to write observation bundle")?;

    Ok(())
}

fn collection_options(args: &PodArgs) -> anyhow::Result<CollectionOptions> {
    Ok(CollectionOptions {
        lookback: chrono::Duration::from_std(args.lookback)
            .context("lookback duration is too large")?,
        tail_lines: args
            .tail_lines
            .try_into()
            .context("tail line count is too large")?,
        include_logs: !args.no_logs,
        include_previous_logs: !args.no_previous_logs,
        include_events: !args.no_events,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn maps_pod_arguments_to_collection_options() {
        let args = PodArgs {
            name: "api-0".to_owned(),
            namespace: "production".to_owned(),
            lookback: Duration::from_secs(30 * 60),
            tail_lines: 250,
            no_logs: true,
            no_previous_logs: true,
            no_events: true,
        };

        let options = collection_options(&args).unwrap();

        assert_eq!(options.lookback, chrono::Duration::minutes(30));
        assert_eq!(options.tail_lines, 250);
        assert!(!options.include_logs);
        assert!(!options.include_previous_logs);
        assert!(!options.include_events);
    }

    #[test]
    fn rejects_tail_count_larger_than_collector_supports() {
        let args = PodArgs {
            name: "api-0".to_owned(),
            namespace: "default".to_owned(),
            lookback: Duration::from_secs(60),
            tail_lines: u64::MAX,
            no_logs: false,
            no_previous_logs: false,
            no_events: false,
        };

        assert!(collection_options(&args).is_err());
    }
}
